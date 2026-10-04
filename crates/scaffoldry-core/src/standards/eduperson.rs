use std::str::FromStr;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EduPersonAffiliation {
    Faculty,
    Student,
    Staff,
    Employee,
    Member,
    Affiliate,
    Alum,
}

impl FromStr for EduPersonAffiliation {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "faculty" => Ok(Self::Faculty),
            "student" => Ok(Self::Student),
            "staff" => Ok(Self::Staff),
            "employee" => Ok(Self::Employee),
            "member" => Ok(Self::Member),
            "affiliate" => Ok(Self::Affiliate),
            "alum" => Ok(Self::Alum),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EduPersonIdentity {
    pub eppn: String,
    pub realm: String,
    pub affiliations: Vec<EduPersonAffiliation>,
}

impl EduPersonIdentity {
    pub fn parse(eppn: &str, scoped_affiliations: Vec<&str>) -> Result<Self, String> {
        let parts: Vec<&str> = eppn.split('@').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            return Err("Invalid ePPN format, expected user@realm".to_string());
        }
        let realm = parts[1].to_string();

        let mut affiliations = Vec::new();
        for scoped in scoped_affiliations {
            let aff_parts: Vec<&str> = scoped.split('@').collect();
            if aff_parts.len() == 2 && aff_parts[1] == realm {
                if let Ok(aff) = EduPersonAffiliation::from_str(aff_parts[0]) {
                    if !affiliations.contains(&aff) {
                        affiliations.push(aff);
                    }
                }
            }
        }

        Ok(Self {
            eppn: eppn.to_string(),
            realm,
            affiliations,
        })
    }

    pub fn has_affiliation(&self, target: EduPersonAffiliation) -> bool {
        self.affiliations.contains(&target)
    }
}
