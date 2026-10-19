use scaffoldry_engine::infer::{infer_column_type, InferredType, AnalyzeOptions};
use std::collections::HashSet;

#[test]
fn test_infer_fixture_20_columns() {
    let active_people: HashSet<&str> = [
        "prof.curie@science.state.edu",
        "student.smith@science.state.edu",
        "dr.watson@science.state.edu",
    ].into_iter().collect();

    let is_person = |s: &str| active_people.contains(s);
    let opts = AnalyzeOptions {
        is_person: Some(is_person),
    };

    // 20 fixture columns:
    // 1. id / integer
    let col1: Vec<&str> = (1..=25).map(|_| "42").collect();
    // 2. decimal / float
    let col2: Vec<&str> = (1..=25).map(|_| "3.14159").collect();
    // 3. negative number
    let col3: Vec<&str> = (1..=25).map(|_| "-100").collect();
    // 4. ISO date
    let col4: Vec<&str> = (1..=25).map(|_| "2026-04-12").collect();
    // 5. ISO datetime Z
    let col5: Vec<&str> = (1..=25).map(|_| "2026-04-12T14:30:00Z").collect();
    // 6. ISO datetime offset
    let col6: Vec<&str> = (1..=25).map(|_| "2026-04-12T14:30:00-04:00").collect();
    // 7. boolean true/false
    let col7: Vec<&str> = (1..=25).map(|i| if i % 2 == 0 { "true" } else { "false" }).collect();
    // 8. boolean yes/no
    let col8: Vec<&str> = (1..=25).map(|i| if i % 2 == 0 { "yes" } else { "no" }).collect();
    // 9. boolean 1/0
    let col9: Vec<&str> = (1..=25).map(|i| if i % 2 == 0 { "1" } else { "0" }).collect();
    // 10. select status (3 values over 25 rows)
    let col10: Vec<&str> = (1..=25).map(|i| match i % 3 { 0 => "Pending", 1 => "Approved", _ => "Rejected" }).collect();
    // 11. select department (4 values over 25 rows)
    let col11: Vec<&str> = (1..=25).map(|i| match i % 4 { 0 => "Bio", 1 => "Chem", 2 => "Phys", _ => "Math" }).collect();
    // 12. email standard
    let col12_strings: Vec<String> = (1..=25).map(|i| format!("user{}@example.com", i)).collect();
    let col12: Vec<&str> = col12_strings.iter().map(|s| s.as_str()).collect();
    // 13. email university
    let col13_strings: Vec<String> = (1..=25).map(|i| format!("scholar{}@state.edu", i)).collect();
    let col13: Vec<&str> = col13_strings.iter().map(|s| s.as_str()).collect();
    // 14. url https
    let col14_strings: Vec<String> = (1..=25).map(|i| format!("https://scaffoldry.io/docs/{}", i)).collect();
    let col14: Vec<&str> = col14_strings.iter().map(|s| s.as_str()).collect();
    // 15. url http
    let col15_strings: Vec<String> = (1..=25).map(|i| format!("http://internal.state.edu/api/{}", i)).collect();
    let col15: Vec<&str> = col15_strings.iter().map(|s| s.as_str()).collect();
    // 16. collaborator 1
    let col16: Vec<&str> = (1..=25).map(|_| "prof.curie@science.state.edu").collect();
    // 17. collaborator 2
    let col17: Vec<&str> = (1..=25).map(|i| if i % 2 == 0 { "student.smith@science.state.edu" } else { "dr.watson@science.state.edu" }).collect();
    // 18. text free form
    let col18: Vec<&str> = (0..25).map(|i| {
        // Unique text strings so distinct > 20
        match i {
            0 => "Alpha memo notes",
            1 => "Beta inspection log",
            2 => "Gamma lab findings",
            3 => "Delta equipment status",
            4 => "Epsilon protocol details",
            5 => "Zeta review comment",
            6 => "Eta incident report",
            7 => "Theta grant summary",
            8 => "Iota curriculum draft",
            9 => "Kappa safety inspection",
            10 => "Lambda budget breakdown",
            11 => "Mu schedule adjustment",
            12 => "Nu milestone reached",
            13 => "Xi risk assessment",
            14 => "Omicron conference notes",
            15 => "Pi audit trail entry",
            16 => "Rho certificate renewal",
            17 => "Sigma meeting transcript",
            18 => "Tau publication draft",
            19 => "Upsilon visitor log",
            20 => "Phi survey response",
            21 => "Chi thesis review",
            22 => "Psi lab booking",
            23 => "Omega closing statement",
            _ => "Final summary remarks",
        }
    }).collect();
    // 19. text with punctuation
    let col19: Vec<&str> = (0..25).map(|i| match i {
        0 => "Item #1 (checked; verified)",
        1 => "Item #2 (checked; verified)",
        2 => "Item #3 (checked; verified)",
        3 => "Item #4 (checked; verified)",
        4 => "Item #5 (checked; verified)",
        5 => "Item #6 (checked; verified)",
        6 => "Item #7 (checked; verified)",
        7 => "Item #8 (checked; verified)",
        8 => "Item #9 (checked; verified)",
        9 => "Item #10 (checked; verified)",
        10 => "Item #11 (checked; verified)",
        11 => "Item #12 (checked; verified)",
        12 => "Item #13 (checked; verified)",
        13 => "Item #14 (checked; verified)",
        14 => "Item #15 (checked; verified)",
        15 => "Item #16 (checked; verified)",
        16 => "Item #17 (checked; verified)",
        17 => "Item #18 (checked; verified)",
        18 => "Item #19 (checked; verified)",
        19 => "Item #20 (checked; verified)",
        20 => "Item #21 (checked; verified)",
        21 => "Item #22 (checked; verified)",
        22 => "Item #23 (checked; verified)",
        23 => "Item #24 (checked; verified)",
        _ => "Item #25 (checked; verified)",
    }).collect();
    // 20. text with mixed formats
    let col20: Vec<&str> = (0..25).map(|i| match i {
        0 => "SKU-99214A",
        1 => "SKU-99214B",
        2 => "SKU-99214C",
        3 => "SKU-99214D",
        4 => "SKU-99214E",
        5 => "SKU-99214F",
        6 => "SKU-99214G",
        7 => "SKU-99214H",
        8 => "SKU-99214I",
        9 => "SKU-99214J",
        10 => "SKU-99214K",
        11 => "SKU-99214L",
        12 => "SKU-99214M",
        13 => "SKU-99214N",
        14 => "SKU-99214O",
        15 => "SKU-99214P",
        16 => "SKU-99214Q",
        17 => "SKU-99214R",
        18 => "SKU-99214S",
        19 => "SKU-99214T",
        20 => "SKU-99214U",
        21 => "SKU-99214V",
        22 => "SKU-99214W",
        23 => "SKU-99214X",
        _ => "SKU-99214Y",
    }).collect();

    assert_eq!(infer_column_type(&col1, &opts), InferredType::Number);
    assert_eq!(infer_column_type(&col2, &opts), InferredType::Number);
    assert_eq!(infer_column_type(&col3, &opts), InferredType::Number);
    assert_eq!(infer_column_type(&col4, &opts), InferredType::Date);
    assert_eq!(infer_column_type(&col5, &opts), InferredType::DateTime);
    assert_eq!(infer_column_type(&col6, &opts), InferredType::DateTime);
    assert_eq!(infer_column_type(&col7, &opts), InferredType::Boolean);
    assert_eq!(infer_column_type(&col8, &opts), InferredType::Boolean);
    assert_eq!(infer_column_type(&col9, &opts), InferredType::Boolean);
    assert_eq!(infer_column_type(&col10, &opts), InferredType::Select);
    assert_eq!(infer_column_type(&col11, &opts), InferredType::Select);
    assert_eq!(infer_column_type(&col12, &opts), InferredType::Email);
    assert_eq!(infer_column_type(&col13, &opts), InferredType::Email);
    assert_eq!(infer_column_type(&col14, &opts), InferredType::Url);
    assert_eq!(infer_column_type(&col15, &opts), InferredType::Url);
    assert_eq!(infer_column_type(&col16, &opts), InferredType::Collaborator);
    assert_eq!(infer_column_type(&col17, &opts), InferredType::Collaborator);
    assert_eq!(infer_column_type(&col18, &opts), InferredType::Text);
    assert_eq!(infer_column_type(&col19, &opts), InferredType::Text);
    assert_eq!(infer_column_type(&col20, &opts), InferredType::Text);
}
