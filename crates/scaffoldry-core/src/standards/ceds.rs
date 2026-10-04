use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CedsDomain {
    PostsecondaryStudent,
    PostsecondaryInstitution,
    Staff,
    AcademicPlan,
    Facility,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CedsElement {
    pub code: String,
    pub name: String,
    pub domain: CedsDomain,
    pub url: String,
    pub sensitivity: String,
}

impl CedsElement {
    pub fn is_ferpa_sensitive(&self) -> bool {
        self.sensitivity == "FERPA_PROTECTED"
    }

    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "000127" => Some(Self {
                code: "000127".to_string(),
                name: "GradePointAverageCumulative".to_string(),
                domain: CedsDomain::PostsecondaryStudent,
                url: "https://ceds.ed.gov/element/000127".to_string(),
                sensitivity: "FERPA_PROTECTED".to_string(),
            }),
            "000128" => Some(Self {
                code: "000128".to_string(),
                name: "HighSchoolGraduationDate".to_string(),
                domain: CedsDomain::PostsecondaryStudent,
                url: "https://ceds.ed.gov/element/000128".to_string(),
                sensitivity: "FERPA_PROTECTED".to_string(),
            }),
            "000569" => Some(Self {
                code: "000569".to_string(),
                name: "FacultyRankType".to_string(),
                domain: CedsDomain::Staff,
                url: "https://ceds.ed.gov/element/000569".to_string(),
                sensitivity: "DIRECTORY".to_string(),
            }),
            _ => None,
        }
    }
}
