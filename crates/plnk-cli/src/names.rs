//! Name resolution and output decoration for the `--names` / `names = true`
//! mode.
//!
//! [`NameIndex`] does two jobs:
//!
//! * **Input** — resolve a name given to a foreign-key argument to the ID the
//!   API needs. Matching is ID-first, then the house three-tier name match.
//! * **Output** — remember ID→name pairs and decorate rendered values with
//!   them: sibling `*Name` fields in JSON, substituted values in table and
//!   markdown output.
//!
//! The mechanism is mapping-driven. `kind_for_field` maps a serde JSON field
//! (for example `boardId`) to a [`NameKind`], and `name_field` derives the
//! sibling name field. Each resource adds its mapping arm and its resolver; the
//! decoration loop itself is shared.

#![allow(clippy::unused_async, clippy::unused_async_trait_impl)] // removed once the first resolver arm lands

use std::collections::{HashMap, HashSet};

use serde::Serialize;
use serde_json::{Map, Value};

use plnk_core::api::{
    BoardApi, CustomFieldApi, CustomFieldGroupApi, LabelApi, ListApi, ProjectApi, UserApi,
};
use plnk_core::error::PlankaError;

/// The kind of resource a foreign-key field points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameKind {
    #[allow(dead_code)] // wired up by the resource commits
    Project,
    #[allow(dead_code)]
    Board,
    #[allow(dead_code)]
    List,
    #[allow(dead_code)]
    User,
    #[allow(dead_code)]
    Label,
    #[allow(dead_code)]
    CustomFieldGroup,
    #[allow(dead_code)]
    BaseCustomFieldGroup,
    #[allow(dead_code)]
    CustomField,
}

/// The bundle of API capabilities the name resolvers need.
///
/// A blanket impl covers any client that implements every resource trait, so
/// commands can keep their existing narrow bounds and just add `NameSource`.
pub trait NameSource:
    ProjectApi + BoardApi + ListApi + UserApi + LabelApi + CustomFieldGroupApi + CustomFieldApi
{
}

impl<T> NameSource for T where
    T: ProjectApi + BoardApi + ListApi + UserApi + LabelApi + CustomFieldGroupApi + CustomFieldApi
{
}

/// ID→name cache plus the enum/search caches the resolvers need.
#[derive(Default)]
pub struct NameIndex {
    enabled: bool,
    names: HashMap<(NameKind, String), String>,
    missing: HashSet<(NameKind, String)>,
    /// Board ID → project ID, learned while resolving boards.
    board_project: HashMap<String, String>,
    /// List ID → board ID, learned while resolving lists.
    list_board: HashMap<String, String>,
}

impl NameIndex {
    /// Create an index. A disabled index performs no lookups and decorates
    /// nothing.
    #[must_use]
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            ..Self::default()
        }
    }

    /// Remember a known ID→name pair.
    fn remember(&mut self, kind: NameKind, id: &str, name: &str) {
        self.names.insert((kind, id.to_string()), name.to_string());
    }

    /// Look up a cached name.
    fn lookup(&self, kind: NameKind, id: &str) -> Option<&str> {
        self.names.get(&(kind, id.to_string())).map(String::as_str)
    }

    /// The JSON field added for a resolved name.
    fn name_field(kind: NameKind) -> &'static str {
        match kind {
            NameKind::Project => "projectName",
            NameKind::Board => "boardName",
            NameKind::List => "listName",
            NameKind::User => "userName",
            NameKind::Label => "labelName",
            NameKind::CustomFieldGroup => "customFieldGroupName",
            NameKind::BaseCustomFieldGroup => "baseCustomFieldGroupName",
            NameKind::CustomField => "customFieldName",
        }
    }

    /// Map a foreign-key JSON field to the resource it points at.
    ///
    /// Each resource commit adds its own arm here.
    fn kind_for_field(_field: &str) -> Option<NameKind> {
        None
    }

    /// Substitute a foreign-key value for display in table/markdown output.
    #[must_use]
    pub(crate) fn display(&self, field: &str, id: &str) -> Option<&str> {
        if !self.enabled {
            return None;
        }
        Self::kind_for_field(field).and_then(|kind| self.lookup(kind, id))
    }

    /// Resolve one ID and cache the result. A resolver returning `None` marks
    /// the ID as unresolvable so it is not retried.
    async fn ensure<C>(&mut self, client: &C, kind: NameKind, id: &str) -> Result<(), PlankaError>
    where
        C: NameSource + ?Sized,
    {
        if !self.enabled
            || self.names.contains_key(&(kind, id.to_string()))
            || self.missing.contains(&(kind, id.to_string()))
        {
            return Ok(());
        }

        match self.resolve_kind(client, kind, id).await? {
            Some(name) => self.remember(kind, id, &name),
            None => {
                self.missing.insert((kind, id.to_string()));
            }
        }
        Ok(())
    }

    /// Resolve a single ID to a name. Each resource commit adds its arm.
    async fn resolve_kind<C>(
        &mut self,
        _client: &C,
        _kind: NameKind,
        _id: &str,
    ) -> Result<Option<String>, PlankaError>
    where
        C: NameSource + ?Sized,
    {
        Ok(None)
    }

    /// Resolve every foreign key reachable in a JSON value.
    pub async fn prefetch_value<C>(&mut self, client: &C, value: &Value) -> Result<(), PlankaError>
    where
        C: NameSource + ?Sized,
    {
        if !self.enabled {
            return Ok(());
        }

        let mut needed = Vec::new();
        collect_foreign_keys(value, &mut needed);
        for (kind, id) in needed {
            self.ensure(client, kind, &id).await?;
        }
        Ok(())
    }

    /// Resolve every foreign key reachable in a serializable item.
    pub async fn prefetch_item<T, C>(&mut self, client: &C, item: &T) -> Result<(), PlankaError>
    where
        T: Serialize,
        C: NameSource + ?Sized,
    {
        if !self.enabled {
            return Ok(());
        }
        let value = serde_json::to_value(item)?;
        self.prefetch_value(client, &value).await
    }

    /// Resolve every foreign key reachable in a collection of items.
    pub async fn prefetch_items<T, C>(&mut self, client: &C, items: &[T]) -> Result<(), PlankaError>
    where
        T: Serialize,
        C: NameSource + ?Sized,
    {
        if !self.enabled {
            return Ok(());
        }
        let value = serde_json::to_value(items)?;
        self.prefetch_value(client, &value).await
    }

    /// Insert sibling `*Name` fields into a JSON value in place.
    pub fn decorate_json(&self, value: &mut Value) {
        if !self.enabled {
            return;
        }
        match value {
            Value::Object(object) => {
                let insertions = self.name_insertions(object);
                for (field, name) in insertions {
                    object.insert(field, Value::String(name));
                }
                for child in object.values_mut() {
                    self.decorate_json(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.decorate_json(item);
                }
            }
            _ => {}
        }
    }

    /// Compute the name fields to add to one JSON object.
    fn name_insertions(&self, object: &Map<String, Value>) -> Vec<(String, String)> {
        let mut insertions = Vec::new();

        for (field, value) in object {
            let Some(kind) = Self::kind_for_field(field) else {
                continue;
            };
            let Some(id) = value.as_str() else {
                continue;
            };
            if let Some(name) = self.lookup(kind, id) {
                let name_field = Self::name_field(kind);
                if !object.contains_key(name_field) {
                    insertions.push((name_field.to_string(), name.to_string()));
                }
            }
        }

        // A list or card carries no `projectId`; derive it from its parent
        // chain so the item still reports the project it belongs to. Only do
        // this once a project-reaching mapping has been registered, so the
        // harness stays inert until a resource commit enables it.
        if !object.contains_key("projectName") && Self::project_mapping_active() {
            if let Some(project_id) = self.project_id_for(object) {
                if let Some(name) = self.lookup(NameKind::Project, &project_id) {
                    insertions.push(("projectName".to_string(), name.to_string()));
                }
            }
        }

        insertions
    }

    /// Whether any registered mapping can resolve a project name.
    fn project_mapping_active() -> bool {
        Self::kind_for_field("projectId").is_some()
            || Self::kind_for_field("boardId").is_some()
            || Self::kind_for_field("listId").is_some()
    }

    /// Walk `projectId` → `boardId` → `listId` to find the owning project.
    fn project_id_for(&self, object: &Map<String, Value>) -> Option<String> {
        if let Some(id) = object.get("projectId").and_then(Value::as_str) {
            return Some(id.to_string());
        }
        if let Some(board_id) = object.get("boardId").and_then(Value::as_str) {
            return self.board_project.get(board_id).cloned();
        }
        if let Some(list_id) = object.get("listId").and_then(Value::as_str) {
            let board_id = self.list_board.get(list_id)?;
            return self.board_project.get(board_id).cloned();
        }
        None
    }
}

/// Recursively collect `(kind, id)` pairs from a JSON value.
fn collect_foreign_keys(value: &Value, out: &mut Vec<(NameKind, String)>) {
    match value {
        Value::Object(object) => {
            for (field, child) in object {
                if let Some(kind) = NameIndex::kind_for_field(field) {
                    if let Some(id) = child.as_str() {
                        out.push((kind, id.to_string()));
                    }
                }
                collect_foreign_keys(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_foreign_keys(item, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_index_decorates_nothing() {
        let index = NameIndex::new(false);
        let mut value = serde_json::json!({ "projectId": "p1" });
        index.decorate_json(&mut value);
        assert!(value.get("projectName").is_none());
    }

    #[test]
    fn enabled_index_decoration_is_mapping_driven() {
        // With no mapping registered, even an in-memory name is not inserted.
        // Resource commits add mappings and cover the positive cases.
        let mut index = NameIndex::new(true);
        index.remember(NameKind::Project, "p1", "Platform");
        let mut value = serde_json::json!({ "projectId": "p1" });
        index.decorate_json(&mut value);
        assert!(value.get("projectName").is_none());
    }
}
