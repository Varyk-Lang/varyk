// An in-memory store with the shapes of varyk-sql's `Pool` and `Tx`: a
// key in place of a query, a JSON value in place of a row, and
// `serde_json::from_value` in place of the row reader. No database.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

type Entries = Arc<Mutex<BTreeMap<String, serde_json::Value>>>;

pub struct Store {
    entries: Entries,
}

pub struct Batch {
    entries: Entries,
    pending: Vec<(String, serde_json::Value)>,
    committed: bool,
}

pub fn open() -> Store {
    Store {
        entries: Arc::new(Mutex::new(BTreeMap::new())),
    }
}

impl Store {
    /// Stores one value as a JSON scalar, and none or several as a JSON
    /// array, under `key`; gives how many values were stored.
    pub fn put(
        &mut self,
        key: &'static str,
        values: Vec<varyk_std::Value>,
    ) -> Result<u64, varyk_std::Error> {
        let count = values.len() as u64;
        let value = entry(values)?;
        locked(&self.entries)?.insert(key.to_string(), value);
        Ok(count)
    }

    /// Reads what is stored under `key` as a `T`; an error when nothing
    /// is. The values are accepted and unused, as varyk-sql binds them.
    pub fn one<T: varyk_std::serde::de::DeserializeOwned>(
        &self,
        key: &'static str,
        values: Vec<varyk_std::Value>,
    ) -> Result<T, varyk_std::Error> {
        match self.first(key, values)? {
            Some(found) => Ok(found),
            None => Err(varyk_std::Error::new(format!("nothing stored under {key}"))),
        }
    }

    /// Reads what is stored under `key` as a `T`, or `None`.
    pub fn first<T: varyk_std::serde::de::DeserializeOwned>(
        &self,
        key: &'static str,
        _values: Vec<varyk_std::Value>,
    ) -> Result<Option<T>, varyk_std::Error> {
        let found = locked(&self.entries)?.get(key).cloned();
        match found {
            Some(value) => read(value).map(Some),
            None => Ok(None),
        }
    }

    /// Reads every entry whose key starts with `prefix` as a `T`, in the
    /// order of their keys; `""` reads them all.
    pub fn all<T: varyk_std::serde::de::DeserializeOwned>(
        &self,
        prefix: &'static str,
        _values: Vec<varyk_std::Value>,
    ) -> Result<Vec<T>, varyk_std::Error> {
        let found: Vec<serde_json::Value> = locked(&self.entries)?
            .iter()
            .filter(|(key, _)| key.starts_with(prefix))
            .map(|(_, value)| value.clone())
            .collect();
        found.into_iter().map(read).collect()
    }

    /// Starts a batch of writes that reach the store on `commit`, and
    /// never when the batch is dropped first.
    pub async fn batch(&self) -> Result<Batch, varyk_std::Error> {
        Ok(Batch {
            entries: Arc::clone(&self.entries),
            pending: Vec::new(),
            committed: false,
        })
    }
}

impl Batch {
    /// As `Store::put`, held back until `commit`.
    pub async fn put(
        &mut self,
        key: &'static str,
        values: Vec<varyk_std::Value>,
    ) -> Result<u64, varyk_std::Error> {
        if self.committed {
            return Err(already_committed());
        }
        let count = values.len() as u64;
        self.pending.push((key.to_string(), entry(values)?));
        Ok(count)
    }

    /// Writes the batch to the store; a second `commit` is an error.
    pub async fn commit(&mut self) -> Result<bool, varyk_std::Error> {
        if self.committed {
            return Err(already_committed());
        }
        let mut entries = locked(&self.entries)?;
        for (key, value) in self.pending.drain(..) {
            entries.insert(key, value);
        }
        self.committed = true;
        Ok(true)
    }
}

fn already_committed() -> varyk_std::Error {
    varyk_std::Error::new("the batch is already committed".to_string())
}

fn locked(
    entries: &Entries,
) -> Result<std::sync::MutexGuard<'_, BTreeMap<String, serde_json::Value>>, varyk_std::Error> {
    entries
        .lock()
        .map_err(|_| varyk_std::Error::new("the store is unusable".to_string()))
}

fn read<T: varyk_std::serde::de::DeserializeOwned>(
    value: serde_json::Value,
) -> Result<T, varyk_std::Error> {
    serde_json::from_value(value).map_err(|err| varyk_std::Error::new(err.to_string()))
}

fn entry(values: Vec<varyk_std::Value>) -> Result<serde_json::Value, varyk_std::Error> {
    let mut json = Vec::new();
    for value in values {
        json.push(scalar(value)?);
    }
    if json.len() == 1 {
        if let Some(only) = json.pop() {
            return Ok(only);
        }
    }
    Ok(serde_json::Value::Array(json))
}

fn scalar(value: varyk_std::Value) -> Result<serde_json::Value, varyk_std::Error> {
    Ok(match value {
        varyk_std::Value::Null => serde_json::Value::Null,
        varyk_std::Value::Bool(b) => serde_json::Value::Bool(b),
        varyk_std::Value::Int(n) => serde_json::Value::from(n),
        varyk_std::Value::Float(x) => match serde_json::Number::from_f64(x) {
            Some(number) => serde_json::Value::Number(number),
            None => {
                return Err(varyk_std::Error::new(format!("cannot store {x}")));
            }
        },
        varyk_std::Value::Text(text) => serde_json::Value::String(text),
        varyk_std::Value::Time(time) => serde_json::Value::String(time.to_iso()),
        varyk_std::Value::Uuid(id) => serde_json::Value::String(id.to_string()),
        varyk_std::Value::Bytes(bytes) => serde_json::Value::String(bytes.to_base64()),
    })
}
