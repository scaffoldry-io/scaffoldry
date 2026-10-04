use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HermDomain {
    LearningAndTeaching,
    Research,
    Engagement,
    StrategyAndGovernance,
    EnablingCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HermCapability {
    pub id: String,
    pub name: String,
    pub domain: HermDomain,
}

impl HermCapability {
    pub fn resolve(id: &str) -> Option<Self> {
        match id {
            "1.2.3" => Some(Self {
                id: "1.2.3".to_string(),
                name: "Curriculum Management".to_string(),
                domain: HermDomain::LearningAndTeaching,
            }),
            "2.2.3" => Some(Self {
                id: "2.2.3".to_string(),
                name: "Research Grant Administration".to_string(),
                domain: HermDomain::Research,
            }),
            "5.3.1" => Some(Self {
                id: "5.3.1".to_string(),
                name: "Space and Facilities Management".to_string(),
                domain: HermDomain::EnablingCapabilities,
            }),
            _ => None,
        }
    }
}
