use crate::ids::Id;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub id: Id,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub start_at: String,
    #[serde(default)]
    pub end_at: Option<String>,
    #[serde(default)]
    pub all_day: bool,
    #[serde(default)]
    pub color: Option<String>,
    /// Polymorphic and unconstrained: `note | task | daily | project | sandbox`.
    #[serde(default)]
    pub ref_type: Option<String>,
    #[serde(default)]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub schedule_id: Option<Id>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Schedule {
    pub id: Id,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
}
