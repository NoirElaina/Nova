//! 有界异步幂等缓存：对写操作（创建会话、启动回合、看板发帖）按 `idempotencyKey` 去重。

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tokio::sync::watch;

const MAX_ENTRIES: usize = 256;
const ENTRY_TTL: Duration = Duration::from_secs(300);

enum Entry {
    InFlight(watch::Receiver<bool>),
    Completed { expires_at: Instant, value: Value },
}

struct InFlightDropGuard<'a> {
    cache: &'a IdempotencyCache,
    cache_key: String,
    done_tx: watch::Sender<bool>,
    settled: bool,
}

impl Drop for InFlightDropGuard<'_> {
    fn drop(&mut self) {
        if !self.settled {
            let mut guard = self.cache.lock_state();
            if matches!(guard.entries.get(&self.cache_key), Some(Entry::InFlight(_))) {
                guard.entries.remove(&self.cache_key);
            }
            drop(guard);
            let _ = self.done_tx.send(true);
        }
    }
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

    /// 执行可返回结果的幂等操作：相同 `idempotency_key` 并发只执行一次并共享同一结果；
    /// 若首次执行失败、被取消或 panic，则自动释放占位并唤醒并发等待者。
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

        let mut flight_guard = loop {
            let mut waiter = None;
            let mut leader_guard = None;
            {
                let now = Instant::now();
                let mut guard = self.lock_state();
                Self::prune_expired(&mut guard, now);
                match guard.entries.get(&cache_key) {
                    Some(Entry::Completed { expires_at, value }) if *expires_at > now => {
                        if let Ok(parsed) = serde_json::from_value::<T>(value.clone()) {
                            return Ok(parsed);
                        }
                    }
                    Some(Entry::InFlight(rx)) => {
                        waiter = Some(rx.clone());
                    }
                    _ => {}
                }
                if waiter.is_none() {
                    let (done_tx, done_rx) = watch::channel(false);
                    guard.entries.insert(cache_key.clone(), Entry::InFlight(done_rx));
                    guard.order.push_back(cache_key.clone());
                    leader_guard = Some(InFlightDropGuard {
                        cache: self,
                        cache_key: cache_key.clone(),
                        done_tx,
                        settled: false,
                    });
                }
            }

            if let Some(guard) = leader_guard {
                break guard;
            }

            if let Some(mut rx) = waiter {
                while !*rx.borrow_and_update() {
                    if rx.changed().await.is_err() {
                        break;
                    }
                }
            }
        };

        let outcome = op().await;
        {
            let mut guard = self.lock_state();
            match outcome {
                Ok(ref value) => match serde_json::to_value(value) {
                    Ok(json) => {
                        guard.entries.insert(
                            cache_key,
                            Entry::Completed {
                                expires_at: Instant::now() + ENTRY_TTL,
                                value: json,
                            },
                        );
                        Self::prune_expired(&mut guard, Instant::now());
                    }
                    Err(_) => {
                        guard.entries.remove(&cache_key);
                    }
                },
                Err(_) => {
                    guard.entries.remove(&cache_key);
                }
            }
        }
        flight_guard.settled = true;
        let _ = flight_guard.done_tx.send(true);
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

    #[tokio::test]
    async fn aborted_leader_releases_inflight_and_wakes_waiter() {
        let cache = Arc::new(IdempotencyCache::default());
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();

        let cache_leader = Arc::clone(&cache);
        let leader = tokio::spawn(async move {
            let _: Result<String, String> = cache_leader
                .run("turn", Some("abort-key"), || async move {
                    let _ = entered_tx.send(());
                    std::future::pending::<Result<String, String>>().await
                })
                .await;
        });
        let _ = entered_rx.await;

        let cache_waiter = Arc::clone(&cache);
        let waiter = tokio::spawn(async move {
            cache_waiter
                .run("turn", Some("abort-key"), || async {
                    Ok::<_, String>("recovered-from-waiter".to_string())
                })
                .await
        });

        tokio::task::yield_now().await;
        leader.abort();

        let res = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("waiter must wake up promptly")
            .expect("waiter join")
            .expect("waiter should recover slot");
        assert_eq!(res, "recovered-from-waiter");
    }

    #[tokio::test]
    async fn concurrent_waiters_all_receive_cached_outcome() {
        let cache = Arc::new(IdempotencyCache::default());
        let runs = Arc::new(AtomicUsize::new(0));
        let (gate_tx, mut gate_rx) = tokio::sync::watch::channel(false);

        let c1 = Arc::clone(&cache);
        let r1 = Arc::clone(&runs);
        let t1 = tokio::spawn(async move {
            c1.run("turn", Some("shared-key"), || async move {
                let n = r1.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = gate_rx.changed().await;
                Ok::<_, String>(format!("val-{n}"))
            })
            .await
        });

        tokio::task::yield_now().await;
        let c2 = Arc::clone(&cache);
        let r2 = Arc::clone(&runs);
        let t2 = tokio::spawn(async move {
            c2.run("turn", Some("shared-key"), || async move {
                r2.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>("duplicate".to_string())
            })
            .await
        });
        let c3 = Arc::clone(&cache);
        let r3 = Arc::clone(&runs);
        let t3 = tokio::spawn(async move {
            c3.run("turn", Some("shared-key"), || async move {
                r3.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>("duplicate".to_string())
            })
            .await
        });

        tokio::task::yield_now().await;
        let _ = gate_tx.send(true);

        assert_eq!(t1.await.expect("t1").expect("ok"), "val-1");
        assert_eq!(t2.await.expect("t2").expect("ok"), "val-1");
        assert_eq!(t3.await.expect("t3").expect("ok"), "val-1");
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }
}
