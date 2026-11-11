//! External JSON data only: no game source checkout, Git or Node dependency.
use serde_json::Value;
use std::{collections::BTreeMap, fs::File, io::Read, path::Path, sync::Arc};

const TABLES: &[&str] = &[
    "config",
    "chess",
    "bonds",
    "garrisons",
    "items",
    "bands",
    "effects",
    "choices",
    "enemies",
    "factions",
    "waves",
    "stages",
    "bosses",
    "tokens",
    "backups",
];

#[derive(Clone, Debug)]
pub struct Definition {
    pub index: u32,
    pub table: String,
    pub id: String,
}

#[derive(Clone, Debug)]
pub struct Catalog {
    tables: BTreeMap<String, Arc<Value>>,
    definitions: Vec<Definition>,
    base_chess: Arc<Value>,
}

impl Catalog {
    /// Budget bounds input bytes, not the in-memory size of the parsed JSON tree.
    /// The future environment allocator must separately account for resident allocations.
    pub fn load(root: &Path, max_bytes: usize) -> Result<Self, String> {
        if max_bytes == 0 {
            return Err("Data byte budget must be positive".into());
        }
        let mut remaining = max_bytes;
        let mut tables = BTreeMap::new();
        for &name in TABLES {
            let path = root.join(format!("{name}.json"));
            let file = File::open(&path).map_err(|e| format!("{name}: {e}"))?;
            let mut bytes = Vec::new();
            file.take((remaining as u64).saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > remaining {
                return Err("Data byte budget exceeded".into());
            }
            remaining -= bytes.len();
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|e| format!("{name}: {e}"))?;
            if !value.is_object() {
                return Err(format!("{name}: expected an object"));
            }
            tables.insert(name.to_string(), Arc::new(value));
        }
        let mut definitions = Vec::new();
        // Stable numeric IDs are independent of file and JSON object insertion order.
        // Simulation iteration must use source order, retained in `tables`, not this index.
        for (name, table) in &tables {
            let mut keys: Vec<_> = table.as_object().unwrap().keys().collect();
            keys.sort();
            for id in keys {
                let index =
                    u32::try_from(definitions.len() + 1).map_err(|_| "Too many definitions")?;
                definitions.push(Definition {
                    index,
                    table: name.clone(),
                    id: id.clone(),
                });
            }
        }
        let base_chess = Arc::clone(&tables["chess"]);
        Ok(Self {
            base_chess,
            tables,
            definitions,
        })
    }
    pub fn table(&self, name: &str) -> Option<&Value> {
        self.tables.get(name).map(Arc::as_ref)
    }
    pub fn definitions(&self) -> &[Definition] {
        &self.definitions
    }
}

mod diy;
