//! 有界异步幂等缓存：对写操作（创建会话、启动回合、看板发帖）按 `idempotencyKey` 去重。

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

const MAX_ENTRIES: usize = 256;
const ENTRY_TTL: Duration = Duration::from_secs(300);

enum Entry {
    InFlight(Arc<Notify>),
    Completed { expires_at: Instant, value: Value },
}

#[derive(Default)]
struct State {
    order: VecDeque<String>,
    entries: HashMap<String, Entry>,
}

/// 进程内有界幂等缓存（线程安全，按 scope + key 隔离，自动过期并限制最大条目数）。
#[derive(Default)]
pub(crate) struct IdempotencyCache {
    state: Mutex<State>,
}

impl IdempotencyCache {
    fn lock_state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn prune_expired(state: &mut State, now: Instant) {
        while let Some(front) = state.order.front() {
            let expired = match state.entries.get(front) {
                Some(Entry::Completed { expires_at, .. }) => *expires_at <= now,
                Some(Entry::InFlight(_)) => false,
                None => true,
            };
            if !expired {
                break;
            }
            if let Some(key) = state.order.pop_front() {
                state.entries.remove(&key);
            }
        }
        let mut budget = state.order.len();
        while state.entries.len() > MAX_ENTRIES && budget > 0 {
            budget -= 1;
            let Some(oldest) = state.order.pop_front() else {
                break;
            };
            if matches!(state.entries.get(&oldest), Some(Entry::InFlight(_))) {
                // 仍在执行中的条目放回队尾，避免被淘汰。
                state.order.push_back(oldest);
            } else {
                state.entries.remove(&oldest);
            }
        }
    }

    fn composite_key(scope: &str, key: &str) -> String {
        format!("{}:{}", scope, key.trim())
    }

    /// 抢占一个“即发即忘（fire-and-forget）”操作的幂等槽位。
    /// 返回 `true` 表示首次调用可执行；返回 `false` 表示近期已受理相同幂等键，调用方应直接返回成功。
    pub(crate) fn claim_once(&self, scope: &str, key: Option<&str>) -> bool {
        let Some(raw) = key.map(str::trim).filter(|k| !k.is_empty()) else {
            return true;
        };
        let cache_key = Self::composite_key(scope, raw);
        let now = Instant::now();
        let mut guard = self.lock_state();
        Self::prune_expired(&mut guard, now);
        if guard.entries.contains_key(&cache_key) {
            return false;
        }
        guard.entries.insert(
            cache_key.clone(),
            Entry::Completed {
                expires_at: now + ENTRY_TTL,
                value: Value::Bool(true),
            },
        );
        guard.order.push_back(cache_key);
        Self::prune_expired(&mut guard, now);
        true
    }

    /// 执行可返回结果的幂等操作：相同 `idempotency_key` 并发只执行一次并共享同一结果；
    /// 若首次执行失败则释放占位允许调用方重试。
    pub(crate) async fn run<T, E, F, Fut>(
        &self,
        scope: &str,
        key: Option<&str>,
        op: F,
    ) -> Result<T, E>
    where
        T: Serialize + DeserializeOwned,
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let Some(raw) = key.map(str::trim).filter(|k| !k.is_empty()) else {
            return op().await;
        };
        let cache_key = Self::composite_key(scope, raw);

        loop {
            let waiter = {
                let now = Instant::now();
                let mut guard = self.lock_state();
                Self::prune_expired(&mut guard, now);
                match guard.entries.get(&cache_key) {
                    Some(Entry::Completed { expires_at, value }) if *expires_at > now => {
                        if let Ok(parsed) = serde_json::from_value::<T>(value.clone()) {
                            return Ok(parsed);
                        }
                        None
                    }
                    Some(Entry::InFlight(notify)) => Some(notify.clone()),
                    _ => {
                        let notify = Arc::new(Notify::new());
                        guard.entries.insert(cache_key.clone(), Entry::InFlight(notify));
                        guard.order.push_back(cache_key.clone());
                        None
                    }
                }
            };

            if let Some(notify) = waiter {
                notify.notified().await;
                continue;
            }
            break;
        }

        let outcome = op().await;
        let mut guard = self.lock_state();
        let waiter = match outcome {
            Ok(ref value) => match serde_json::to_value(value) {
                Ok(json) => {
                    let prev = guard.entries.insert(
                        cache_key,
                        Entry::Completed {
                            expires_at: Instant::now() + ENTRY_TTL,
                            value: json,
                        },
                    );
                    Self::prune_expired(&mut guard, Instant::now());
                    prev
                }
                Err(_) => guard.entries.remove(&cache_key),
            },
            Err(_) => guard.entries.remove(&cache_key),
        };
        if let Some(Entry::InFlight(notify)) = waiter {
            notify.notify_waiters();
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::IdempotencyCache;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn deduplicates_success_and_allows_retry_after_error() {
        let cache = IdempotencyCache::default();
        let runs = Arc::new(AtomicUsize::new(0));

        // 首次失败：不应缓存错误，允许用同一个 key 重试
        let first: Result<String, String> = cache
            .run("create_conversation", Some("k-1"), || async {
                runs.fetch_add(1, Ordering::SeqCst);
                Err("transient".to_string())
            })
            .await;
        assert!(first.is_err());

        // 重试成功并缓存
        let second: Result<String, String> = cache
            .run("create_conversation", Some("k-1"), || async {
                let n = runs.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(format!("conv-{}", n))
            })
            .await;
        assert_eq!(second.expect("ok"), "conv-2");

        // 相同 key 重放：直接命中缓存，不再次递增计数
        let third: Result<String, String> = cache
            .run("create_conversation", Some("k-1"), || async {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok("should-not-run".to_string())
            })
            .await;
        assert_eq!(third.expect("cached"), "conv-2");
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn claim_once_blocks_duplicate_keys_and_allows_blank_keys() {
        let cache = IdempotencyCache::default();
        assert!(cache.claim_once("turn", Some("abc")));
        assert!(!cache.claim_once("turn", Some("abc")));
        assert!(cache.claim_once("turn", None));
        assert!(cache.claim_once("turn", Some("  ")));
    }
}
