use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    Free,
    Paid,
}

impl Plan {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Plan::Free => "free",
            Plan::Paid => "paid",
        }
    }

    pub fn from_db_str(raw: &str) -> Self {
        match raw {
            "paid" => Plan::Paid,
            _ => Plan::Free,
        }
    }
}

/// Returned by `GET /v1/account/plan`. Lives here (not the route module) so
/// the same shape can power both the API and any internal logic that needs
/// the limits in code.
#[derive(Serialize)]
pub struct PlanInfo {
    pub plan: Plan,
    pub retention_days: i64,
    pub max_storage_mb: i64,
    pub sync: bool,
}

impl PlanInfo {
    pub fn for_plan(plan: Plan) -> Self {
        match plan {
            Plan::Free => PlanInfo {
                plan,
                retention_days: 7,
                max_storage_mb: 25,
                sync: false,
            },
            Plan::Paid => PlanInfo {
                plan,
                // Sentinel: -1 = unlimited; clients special-case.
                retention_days: -1,
                max_storage_mb: 5 * 1024,
                sync: true,
            },
        }
    }
}
