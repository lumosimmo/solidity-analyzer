use std::collections::HashMap;

use sa_base_db::FileId;
use sa_intern::{InternId, Interner};
use sa_span::TextRange;
use sa_syntax::Parse;
use solar_ast::{Ident, ItemKind, SourceUnit};
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractId(InternId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(InternId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructId(InternId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumId(InternId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventId(InternId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorId(InternId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModifierId(InternId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VariableId(InternId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UdvtId(InternId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefId {
    Contract(ContractId),
    Function(FunctionId),
    Struct(StructId),
    Enum(EnumId),
    Event(EventId),
    Error(ErrorId),
    Modifier(ModifierId),
    Variable(VariableId),
    Udvt(UdvtId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefKind {
    Contract,
    Function,
    Struct,
    Enum,
    Event,
    Error,
    Modifier,
    Variable,
    Udvt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefLocation {
    file_id: FileId,
    name: String,
    range: TextRange,
}

impl DefLocation {
    pub fn file_id(&self) -> FileId {
        self.file_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn range(&self) -> TextRange {
        self.range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefEntry {
    id: DefId,
    kind: DefKind,
    location: DefLocation,
    container: Option<String>,
}

impl DefEntry {
    pub fn id(&self) -> DefId {
        self.id
    }

    pub fn kind(&self) -> DefKind {
        self.kind
    }

    pub fn location(&self) -> &DefLocation {
        &self.location
    }

    pub fn container(&self) -> Option<&str> {
        self.container.as_deref()
    }
}

#[derive(Debug, Clone, Default)]
pub struct DefMap {
    entries: Vec<DefEntry>,
    index: HashMap<DefId, usize>,
    name_index: HashMap<DefNameKey, Vec<usize>>,
    file_name_index: HashMap<FileNameKey, Vec<usize>>,
}

impl PartialEq for DefMap {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}

impl Eq for DefMap {}

impl DefMap {
    pub fn entries(&self) -> &[DefEntry] {
        &self.entries
    }

    pub fn entry(&self, id: DefId) -> Option<&DefEntry> {
        self.index.get(&id).and_then(|idx| self.entries.get(*idx))
    }

    pub fn entry_by_name(&self, kind: DefKind, name: &str) -> Option<&DefEntry> {
        self.entries_by_name(kind, name)
            .and_then(|entries| entries.into_iter().next())
    }

    pub fn entries_by_name(&self, kind: DefKind, name: &str) -> Option<Vec<&DefEntry>> {
        let key = DefNameKey {
            kind,
            name: name.to_string(),
        };
        self.name_index.get(&key).map(|indices| {
            indices
                .iter()
                .filter_map(|idx| self.entries.get(*idx))
                .collect()
        })
    }

    pub fn entries_by_name_in_file(&self, file_id: FileId, name: &str) -> Vec<&DefEntry> {
        let key = FileNameKey {
            file_id,
            name: name.to_string(),
        };
        let Some(indices) = self.file_name_index.get(&key) else {
            return Vec::new();
        };
        indices
            .iter()
            .filter_map(|idx| self.entries.get(*idx))
            .collect()
    }

    pub fn entries_by_name_in_container(
        &self,
        kind: DefKind,
        name: &str,
        container: Option<&str>,
    ) -> Vec<&DefEntry> {
        self.entries_by_name(kind, name)
            .map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| entry.container.as_deref() == container)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn entry_by_name_in_container(
        &self,
        kind: DefKind,
        name: &str,
        container: Option<&str>,
    ) -> Option<&DefEntry> {
        self.entries_by_name_in_container(kind, name, container)
            .into_iter()
            .next()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DefNameKey {
    kind: DefKind,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileNameKey {
    file_id: FileId,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ContractKey {
    file_id: FileId,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FunctionKey {
    file_id: FileId,
    name: String,
    container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct StructKey {
    file_id: FileId,
    name: String,
    container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EnumKey {
    file_id: FileId,
    name: String,
    container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EventKey {
    file_id: FileId,
    name: String,
    container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ErrorKey {
    file_id: FileId,
    name: String,
    container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ModifierKey {
    file_id: FileId,
    name: String,
    container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct VariableKey {
    file_id: FileId,
    name: String,
    container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct UdvtKey {
    file_id: FileId,
    name: String,
    container: Option<String>,
}

#[derive(Debug, Default)]
pub struct DefDatabase {
    contract_interner: Interner<ContractKey>,
    function_interner: Interner<FunctionKey>,
    struct_interner: Interner<StructKey>,
    enum_interner: Interner<EnumKey>,
    event_interner: Interner<EventKey>,
    error_interner: Interner<ErrorKey>,
    modifier_interner: Interner<ModifierKey>,
    variable_interner: Interner<VariableKey>,
    udvt_interner: Interner<UdvtKey>,
}

impl DefDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn collect<'a>(&mut self, files: impl IntoIterator<Item = (FileId, &'a str)>) -> DefMap {
        let mut map = DefMap::default();

        for (file_id, text) in files {
            let parse = sa_syntax::parse_file(text);
            let tree = parse.tree();
            self.collect_source_unit(&parse, file_id, tree, &mut map);
        }

        map
    }

    fn collect_source_unit(
        &mut self,
        parse: &Parse,
        file_id: FileId,
        unit: &SourceUnit<'_>,
        map: &mut DefMap,
    ) {
        for item in unit.items.iter() {
            match &item.kind {
                ItemKind::Contract(contract) => {
                    let ident = contract.name;
                    let name = ident_text(parse, ident);
                    let range = ident_range(parse, ident);
                    if let Some(range) = range {
                        let id = self.intern_contract(file_id, &name);
                        map.insert_entry(DefEntry {
                            id: DefId::Contract(id),
                            kind: DefKind::Contract,
                            location: DefLocation {
                                file_id,
                                name: name.clone(),
                                range,
                            },
                            container: None,
                        });
                        self.collect_contract_items(parse, file_id, &name, &contract.body, map);
                    }
                }
                ItemKind::Function(function) => {
                    if let Some(ident) = function.header.name {
                        let name = ident_text(parse, ident);
                        let Some(range) = ident_range(parse, ident) else {
                            continue;
                        };
                        match function.kind {
                            solar_ast::FunctionKind::Modifier => {
                                let id = self.intern_modifier(file_id, &name, None);
                                map.insert_entry(DefEntry {
                                    id: DefId::Modifier(id),
                                    kind: DefKind::Modifier,
                                    location: DefLocation {
                                        file_id,
                                        name: name.clone(),
                                        range,
                                    },
                                    container: None,
                                });
                            }
                            _ => {
                                let id = self.intern_function(file_id, &name, None);
                                map.insert_entry(DefEntry {
                                    id: DefId::Function(id),
                                    kind: DefKind::Function,
                                    location: DefLocation {
                                        file_id,
                                        name: name.clone(),
                                        range,
                                    },
                                    container: None,
                                });
                            }
                        }
                    }
                }
                ItemKind::Variable(item) => {
                    if let Some(ident) = item.name {
                        let name = ident_text(parse, ident);
                        let Some(range) = ident_range(parse, ident) else {
                            continue;
                        };
                        let id = self.intern_variable(file_id, &name, None);
                        map.insert_entry(DefEntry {
                            id: DefId::Variable(id),
                            kind: DefKind::Variable,
                            location: DefLocation {
                                file_id,
                                name: name.clone(),
                                range,
                            },
                            container: None,
                        });
                    }
                }
                ItemKind::Struct(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_struct(file_id, &name, None);
                    map.insert_entry(DefEntry {
                        id: DefId::Struct(id),
                        kind: DefKind::Struct,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: None,
                    });
                }
                ItemKind::Enum(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_enum(file_id, &name, None);
                    map.insert_entry(DefEntry {
                        id: DefId::Enum(id),
                        kind: DefKind::Enum,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: None,
                    });
                }
                ItemKind::Event(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_event(file_id, &name, None);
                    map.insert_entry(DefEntry {
                        id: DefId::Event(id),
                        kind: DefKind::Event,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: None,
                    });
                }
                ItemKind::Error(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_error(file_id, &name, None);
                    map.insert_entry(DefEntry {
                        id: DefId::Error(id),
                        kind: DefKind::Error,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: None,
                    });
                }
                ItemKind::Udvt(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_udvt(file_id, &name, None);
                    map.insert_entry(DefEntry {
                        id: DefId::Udvt(id),
                        kind: DefKind::Udvt,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: None,
                    });
                }
                _ => {}
            }
        }
    }

    fn collect_contract_items(
        &mut self,
        parse: &Parse,
        file_id: FileId,
        contract_name: &str,
        items: &solar_ast::BoxSlice<'_, solar_ast::Item<'_>>,
        map: &mut DefMap,
    ) {
        for item in items.iter() {
            match &item.kind {
                ItemKind::Function(function) => {
                    if let Some(ident) = function.header.name {
                        let name = ident_text(parse, ident);
                        let Some(range) = ident_range(parse, ident) else {
                            continue;
                        };
                        match function.kind {
                            solar_ast::FunctionKind::Modifier => {
                                let id = self.intern_modifier(file_id, &name, Some(contract_name));
                                map.insert_entry(DefEntry {
                                    id: DefId::Modifier(id),
                                    kind: DefKind::Modifier,
                                    location: DefLocation {
                                        file_id,
                                        name: name.clone(),
                                        range,
                                    },
                                    container: Some(contract_name.to_string()),
                                });
                            }
                            _ => {
                                let id = self.intern_function(file_id, &name, Some(contract_name));
                                map.insert_entry(DefEntry {
                                    id: DefId::Function(id),
                                    kind: DefKind::Function,
                                    location: DefLocation {
                                        file_id,
                                        name: name.clone(),
                                        range,
                                    },
                                    container: Some(contract_name.to_string()),
                                });
                            }
                        }
                    }
                }
                ItemKind::Variable(item) => {
                    if let Some(ident) = item.name {
                        let name = ident_text(parse, ident);
                        let Some(range) = ident_range(parse, ident) else {
                            continue;
                        };
                        let id = self.intern_variable(file_id, &name, Some(contract_name));
                        map.insert_entry(DefEntry {
                            id: DefId::Variable(id),
                            kind: DefKind::Variable,
                            location: DefLocation {
                                file_id,
                                name: name.clone(),
                                range,
                            },
                            container: Some(contract_name.to_string()),
                        });
                    }
                }
                ItemKind::Struct(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_struct(file_id, &name, Some(contract_name));
                    map.insert_entry(DefEntry {
                        id: DefId::Struct(id),
                        kind: DefKind::Struct,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: Some(contract_name.to_string()),
                    });
                }
                ItemKind::Enum(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_enum(file_id, &name, Some(contract_name));
                    map.insert_entry(DefEntry {
                        id: DefId::Enum(id),
                        kind: DefKind::Enum,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: Some(contract_name.to_string()),
                    });
                }
                ItemKind::Event(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_event(file_id, &name, Some(contract_name));
                    map.insert_entry(DefEntry {
                        id: DefId::Event(id),
                        kind: DefKind::Event,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: Some(contract_name.to_string()),
                    });
                }
                ItemKind::Error(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_error(file_id, &name, Some(contract_name));
                    map.insert_entry(DefEntry {
                        id: DefId::Error(id),
                        kind: DefKind::Error,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: Some(contract_name.to_string()),
                    });
                }
                ItemKind::Udvt(item) => {
                    let ident = item.name;
                    let name = ident_text(parse, ident);
                    let Some(range) = ident_range(parse, ident) else {
                        continue;
                    };
                    let id = self.intern_udvt(file_id, &name, Some(contract_name));
                    map.insert_entry(DefEntry {
                        id: DefId::Udvt(id),
                        kind: DefKind::Udvt,
                        location: DefLocation {
                            file_id,
                            name: name.clone(),
                            range,
                        },
                        container: Some(contract_name.to_string()),
                    });
                }
                _ => {}
            }
        }
    }

    fn intern_contract(&mut self, file_id: FileId, name: &str) -> ContractId {
        let key = ContractKey {
            file_id,
            name: name.to_string(),
        };
        ContractId(self.contract_interner.intern(key))
    }

    fn intern_function(
        &mut self,
        file_id: FileId,
        name: &str,
        container: Option<&str>,
    ) -> FunctionId {
        let key = FunctionKey {
            file_id,
            name: name.to_string(),
            container: container.map(ToString::to_string),
        };
        FunctionId(self.function_interner.intern(key))
    }

    fn intern_struct(&mut self, file_id: FileId, name: &str, container: Option<&str>) -> StructId {
        let key = StructKey {
            file_id,
            name: name.to_string(),
            container: container.map(ToString::to_string),
        };
        StructId(self.struct_interner.intern(key))
    }

    fn intern_enum(&mut self, file_id: FileId, name: &str, container: Option<&str>) -> EnumId {
        let key = EnumKey {
            file_id,
            name: name.to_string(),
            container: container.map(ToString::to_string),
        };
        EnumId(self.enum_interner.intern(key))
    }

    fn intern_event(&mut self, file_id: FileId, name: &str, container: Option<&str>) -> EventId {
        let key = EventKey {
            file_id,
            name: name.to_string(),
            container: container.map(ToString::to_string),
        };
        EventId(self.event_interner.intern(key))
    }

    fn intern_error(&mut self, file_id: FileId, name: &str, container: Option<&str>) -> ErrorId {
        let key = ErrorKey {
            file_id,
            name: name.to_string(),
            container: container.map(ToString::to_string),
        };
        ErrorId(self.error_interner.intern(key))
    }

    fn intern_modifier(
        &mut self,
        file_id: FileId,
        name: &str,
        container: Option<&str>,
    ) -> ModifierId {
        let key = ModifierKey {
            file_id,
            name: name.to_string(),
            container: container.map(ToString::to_string),
        };
        ModifierId(self.modifier_interner.intern(key))
    }

    fn intern_variable(
        &mut self,
        file_id: FileId,
        name: &str,
        container: Option<&str>,
    ) -> VariableId {
        let key = VariableKey {
            file_id,
            name: name.to_string(),
            container: container.map(ToString::to_string),
        };
        VariableId(self.variable_interner.intern(key))
    }

    fn intern_udvt(&mut self, file_id: FileId, name: &str, container: Option<&str>) -> UdvtId {
        let key = UdvtKey {
            file_id,
            name: name.to_string(),
            container: container.map(ToString::to_string),
        };
        UdvtId(self.udvt_interner.intern(key))
    }
}

impl DefMap {
    fn insert_entry(&mut self, entry: DefEntry) {
        let id = entry.id;
        let idx = self.entries.len();
        let name_key = DefNameKey {
            kind: entry.kind,
            name: entry.location.name.clone(),
        };
        let file_name_key = FileNameKey {
            file_id: entry.location.file_id,
            name: entry.location.name.clone(),
        };
        self.entries.push(entry);
        self.index.insert(id, idx);
        self.name_index.entry(name_key).or_default().push(idx);
        self.file_name_index
            .entry(file_name_key)
            .or_default()
            .push(idx);
    }
}

fn ident_text(parse: &Parse, ident: Ident) -> String {
    parse.with_session(|| ident.as_str().to_string())
}

fn ident_range(parse: &Parse, ident: Ident) -> Option<TextRange> {
    let range = parse.span_to_text_range(ident.span);
    if range.is_none() {
        warn!(
            ?ident,
            span = ?ident.span,
            "failed to convert ident span to TextRange"
        );
    }
    range
}

#[cfg(test)]
mod tests {
    use super::{DefDatabase, DefKind};
    use sa_base_db::FileId;

    #[test]
    fn stable_ids_for_top_level_items() {
        let mut db = DefDatabase::new();
        let file_id = FileId::from_raw(0);

        let before = db.collect([(
            file_id,
            "contract Foo { function bar() public { uint256 x = 1; } }",
        )]);
        let foo_before = before
            .entry_by_name(DefKind::Contract, "Foo")
            .expect("contract Foo");
        let bar_before = before
            .entry_by_name(DefKind::Function, "bar")
            .expect("function bar");

        let after = db.collect([(
            file_id,
            "contract Foo { function bar() public { uint256 x = 2; } }",
        )]);
        let foo_after = after
            .entry_by_name(DefKind::Contract, "Foo")
            .expect("contract Foo");
        let bar_after = after
            .entry_by_name(DefKind::Function, "bar")
            .expect("function bar");

        assert_eq!(foo_before.id(), foo_after.id());
        assert_eq!(bar_before.id(), bar_after.id());
    }

    #[test]
    fn ids_differ_for_same_name_in_different_files() {
        let mut db = DefDatabase::new();
        let file_a = FileId::from_raw(0);
        let file_b = FileId::from_raw(1);

        let map = db.collect([(file_a, "contract Foo {}"), (file_b, "contract Foo {}")]);

        let mut ids = map
            .entries()
            .iter()
            .filter(|entry| entry.kind() == DefKind::Contract)
            .map(|entry| entry.id())
            .collect::<Vec<_>>();
        ids.sort_by_key(|id| format!("{id:?}"));
        ids.dedup();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn missing_names_return_none() {
        let mut db = DefDatabase::new();
        let file_id = FileId::from_raw(0);
        let map = db.collect([(file_id, "contract Foo {}")]);

        assert!(map.entry_by_name(DefKind::Contract, "Bar").is_none());
        assert!(map.entries_by_name(DefKind::Contract, "Bar").is_none());
    }

    #[test]
    fn indexes_events_errors_modifiers_and_variables() {
        let mut db = DefDatabase::new();
        let file_id = FileId::from_raw(0);

        let map = db.collect([(
            file_id,
            "type Price is uint256;\
             event TopEvent();\
             error TopError();\
             contract Foo {\
                 struct Thing { uint256 value; }\
                 enum Kind { A }\
                 event InnerEvent();\
                 error InnerError();\
                 modifier OnlyOwner() { _; }\
                 uint256 value;\
             }",
        )]);

        assert!(map.entry_by_name(DefKind::Event, "TopEvent").is_some());
        assert!(map.entry_by_name(DefKind::Error, "TopError").is_some());
        assert!(map.entry_by_name(DefKind::Event, "InnerEvent").is_some());
        assert!(map.entry_by_name(DefKind::Error, "InnerError").is_some());
        assert!(map.entry_by_name(DefKind::Modifier, "OnlyOwner").is_some());
        assert!(map.entry_by_name(DefKind::Variable, "value").is_some());
        assert!(map.entry_by_name(DefKind::Struct, "Thing").is_some());
        assert!(map.entry_by_name(DefKind::Enum, "Kind").is_some());
        assert!(map.entry_by_name(DefKind::Udvt, "Price").is_some());

        assert_eq!(
            map.entries_by_name_in_container(DefKind::Event, "TopEvent", None)
                .len(),
            1
        );
        assert_eq!(
            map.entries_by_name_in_container(DefKind::Event, "InnerEvent", Some("Foo"))
                .len(),
            1
        );
        assert_eq!(
            map.entries_by_name_in_container(DefKind::Error, "TopError", None)
                .len(),
            1
        );
        assert_eq!(
            map.entries_by_name_in_container(DefKind::Error, "InnerError", Some("Foo"))
                .len(),
            1
        );
        assert_eq!(
            map.entries_by_name_in_container(DefKind::Modifier, "OnlyOwner", Some("Foo"))
                .len(),
            1
        );
        assert_eq!(
            map.entries_by_name_in_container(DefKind::Variable, "value", Some("Foo"))
                .len(),
            1
        );
    }

    #[test]
    fn container_scoped_entries_are_distinct() {
        let mut db = DefDatabase::new();
        let file_id = FileId::from_raw(0);

        let map = db.collect([(
            file_id,
            "function bar() {}\
             contract Foo {\
                 function bar() {}\
                 struct Thing { uint256 value; }\
                 enum Kind { A }\
                 event Ev();\
                 error Err();\
                 modifier OnlyOwner() { _; }\
                 uint256 value;\
             }\
             contract Bar {\
                 function bar() {}\
                 struct Thing { uint256 value; }\
                 enum Kind { A }\
                 event Ev();\
                 error Err();\
                 modifier OnlyOwner() { _; }\
             }",
        )]);

        let top_bar = map
            .entry_by_name_in_container(DefKind::Function, "bar", None)
            .expect("top-level bar");
        let foo_bar = map
            .entry_by_name_in_container(DefKind::Function, "bar", Some("Foo"))
            .expect("Foo::bar");
        let bar_bar = map
            .entry_by_name_in_container(DefKind::Function, "bar", Some("Bar"))
            .expect("Bar::bar");

        assert_ne!(top_bar.id(), foo_bar.id());
        assert_ne!(foo_bar.id(), bar_bar.id());
        assert_eq!(top_bar.container(), None);
        assert_eq!(foo_bar.container(), Some("Foo"));
        assert_eq!(bar_bar.container(), Some("Bar"));

        let foo_struct = map
            .entry_by_name_in_container(DefKind::Struct, "Thing", Some("Foo"))
            .expect("Foo::Thing");
        let bar_struct = map
            .entry_by_name_in_container(DefKind::Struct, "Thing", Some("Bar"))
            .expect("Bar::Thing");
        assert_ne!(foo_struct.id(), bar_struct.id());

        let foo_enum = map
            .entry_by_name_in_container(DefKind::Enum, "Kind", Some("Foo"))
            .expect("Foo::Kind");
        let bar_enum = map
            .entry_by_name_in_container(DefKind::Enum, "Kind", Some("Bar"))
            .expect("Bar::Kind");
        assert_ne!(foo_enum.id(), bar_enum.id());

        let foo_event = map
            .entry_by_name_in_container(DefKind::Event, "Ev", Some("Foo"))
            .expect("Foo::Ev");
        let bar_event = map
            .entry_by_name_in_container(DefKind::Event, "Ev", Some("Bar"))
            .expect("Bar::Ev");
        assert_ne!(foo_event.id(), bar_event.id());

        let foo_error = map
            .entry_by_name_in_container(DefKind::Error, "Err", Some("Foo"))
            .expect("Foo::Err");
        let bar_error = map
            .entry_by_name_in_container(DefKind::Error, "Err", Some("Bar"))
            .expect("Bar::Err");
        assert_ne!(foo_error.id(), bar_error.id());

        let foo_modifier = map
            .entry_by_name_in_container(DefKind::Modifier, "OnlyOwner", Some("Foo"))
            .expect("Foo::OnlyOwner");
        let bar_modifier = map
            .entry_by_name_in_container(DefKind::Modifier, "OnlyOwner", Some("Bar"))
            .expect("Bar::OnlyOwner");
        assert_ne!(foo_modifier.id(), bar_modifier.id());

        let foo_var = map
            .entry_by_name_in_container(DefKind::Variable, "value", Some("Foo"))
            .expect("Foo::value");
        assert_eq!(foo_var.container(), Some("Foo"));
    }

    #[test]
    fn entries_by_name_in_file_filters_by_file() {
        let mut db = DefDatabase::new();
        let file_a = FileId::from_raw(0);
        let file_b = FileId::from_raw(1);

        let map = db.collect([
            (
                file_a,
                "type Price is uint256;\
                 contract Foo {\
                     function bar() {}\
                     event Ev();\
                 }",
            ),
            (
                file_b,
                "type Price is uint256;\
                 contract Foo {\
                     function bar() {}\
                     event Ev();\
                 }",
            ),
        ]);

        let foo_a = map
            .entries_by_name_in_file(file_a, "Foo")
            .into_iter()
            .find(|entry| entry.kind() == DefKind::Contract)
            .expect("Foo in file A");
        let foo_b = map
            .entries_by_name_in_file(file_b, "Foo")
            .into_iter()
            .find(|entry| entry.kind() == DefKind::Contract)
            .expect("Foo in file B");
        assert_ne!(foo_a.id(), foo_b.id());

        let bar_a = map
            .entries_by_name_in_file(file_a, "bar")
            .into_iter()
            .find(|entry| entry.kind() == DefKind::Function)
            .expect("bar in file A");
        let bar_b = map
            .entries_by_name_in_file(file_b, "bar")
            .into_iter()
            .find(|entry| entry.kind() == DefKind::Function)
            .expect("bar in file B");
        assert_ne!(bar_a.id(), bar_b.id());

        let price_a = map
            .entries_by_name_in_file(file_a, "Price")
            .into_iter()
            .find(|entry| entry.kind() == DefKind::Udvt)
            .expect("Price in file A");
        let price_b = map
            .entries_by_name_in_file(file_b, "Price")
            .into_iter()
            .find(|entry| entry.kind() == DefKind::Udvt)
            .expect("Price in file B");
        assert_ne!(price_a.id(), price_b.id());
    }

    #[test]
    fn entry_accessors_and_indexes_cover_multiple_paths() {
        let mut db = DefDatabase::new();
        let file_id = FileId::from_raw(0);

        let map = db.collect([(
            file_id,
            "contract Foo { function bar() {} } function bar() {}",
        )]);

        let foo = map
            .entry_by_name(DefKind::Contract, "Foo")
            .expect("Foo contract");
        let foo_by_id = map.entry(foo.id()).expect("entry by id");
        let location = foo_by_id.location();
        assert_eq!(location.file_id(), file_id);
        assert_eq!(location.name(), "Foo");
        assert!(!location.range().is_empty());
        assert_eq!(foo_by_id.container(), None);

        let bars = map
            .entries_by_name(DefKind::Function, "bar")
            .expect("bar entries");
        assert_eq!(bars.len(), 2);

        assert!(
            map.entry_by_name_in_container(DefKind::Function, "missing", None)
                .is_none()
        );
        assert!(
            map.entries_by_name_in_container(DefKind::Function, "bar", Some("Missing"))
                .is_empty()
        );
        assert!(
            map.entries_by_name_in_file(FileId::from_raw(99), "Foo")
                .is_empty()
        );
    }
}
