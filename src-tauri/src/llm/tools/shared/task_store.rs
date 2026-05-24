use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: u64,
    pub title: String,
    pub status: String,
    pub notes: Option<String>,
}

#[derive(Debug, Default)]
struct ConversationTasks {
    next_id: u64,
    items: Vec<TodoItem>,
}

static TASKS: OnceLock<Mutex<HashMap<String, ConversationTasks>>> = OnceLock::new();

fn tasks_store() -> &'static Mutex<HashMap<String, ConversationTasks>> {
    TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn scope_key(conversation_id: Option<&str>) -> String {
    conversation_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("__default__")
        .to_string()
}

fn next_id(scope: &mut ConversationTasks) -> u64 {
    if scope.next_id == 0 {
        scope.next_id = 1;
    }
    let id = scope.next_id;
    scope.next_id += 1;
    id
}

pub fn list(conversation_id: Option<&str>) -> Vec<TodoItem> {
    let key = scope_key(conversation_id);
    tasks_store()
        .lock()
        .expect("TASKS mutex poisoned")
        .get(&key)
        .map(|scope| scope.items.clone())
        .unwrap_or_default()
}

pub fn create(
    conversation_id: Option<&str>,
    title: String,
    status: String,
    notes: Option<String>,
) -> TodoItem {
    let key = scope_key(conversation_id);
    let mut store = tasks_store().lock().expect("TASKS mutex poisoned");
    let scope = store.entry(key).or_default();
    let item = TodoItem {
        id: next_id(scope),
        title,
        status,
        notes,
    };
    scope.items.push(item.clone());
    item
}

pub fn update(
    conversation_id: Option<&str>,
    id: u64,
    title: Option<String>,
    status: Option<String>,
    notes: Option<Option<String>>,
) -> Option<TodoItem> {
    let key = scope_key(conversation_id);
    let mut store = tasks_store().lock().expect("TASKS mutex poisoned");
    let task = store
        .get_mut(&key)?
        .items
        .iter_mut()
        .find(|task| task.id == id)?;

    if let Some(title) = title {
        task.title = title;
    }
    if let Some(status) = status {
        task.status = status;
    }
    if let Some(notes) = notes {
        task.notes = notes;
    }

    Some(task.clone())
}

pub fn get(conversation_id: Option<&str>, id: u64) -> Option<TodoItem> {
    let key = scope_key(conversation_id);
    tasks_store()
        .lock()
        .expect("TASKS mutex poisoned")
        .get(&key)?
        .items
        .iter()
        .find(|task| task.id == id)
        .cloned()
}

pub fn replace_all(
    conversation_id: Option<&str>,
    items: Vec<(String, String, Option<String>)>,
) -> Vec<TodoItem> {
    let key = scope_key(conversation_id);
    let mut store = tasks_store().lock().expect("TASKS mutex poisoned");
    let scope = store.entry(key).or_default();
    scope.items.clear();
    scope.next_id = 1;

    let mut created = Vec::with_capacity(items.len());
    for (title, status, notes) in items {
        let item = TodoItem {
            id: next_id(scope),
            title,
            status,
            notes,
        };
        scope.items.push(item.clone());
        created.push(item);
    }
    created
}
