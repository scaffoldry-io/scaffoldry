//! Stateless server phase 3, test 5: two moves at once cannot make a cycle. The check and the move are one
//! transaction that locks the unit, so the second request finds the tree the first one left.

mod common;

use common::TestDb;
use scaffoldry_server::state::OrganizationNode;
use uuid::Uuid;

const ROOT: &str = "00000000-0000-0000-0000-000000000001";

// 5. Two requests at once move X under Y and Y under X. One wins, the other is refused, and the tree has no cycle.
#[test]
fn two_opposite_moves_at_once_leave_no_cycle() {
    let db = TestDb::new();
    let state = db.state();
    let root = Uuid::parse_str(ROOT).unwrap();
    for round in 0..8 {
        let unit = |code: String| {
            let node = OrganizationNode { id: Uuid::new_v4(), parent_id: Some(root), name: code.clone(), code, org_type: "Department".into() };
            state.persist_organization(&node).unwrap();
            node
        };
        let (x, y) = (unit(format!("X{round}")), unit(format!("Y{round}")));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let results: Vec<_> = [(x.clone(), y.id), (y.clone(), x.id)]
            .into_iter()
            .map(|(mut node, under)| {
                let (state, barrier) = (db.state(), barrier.clone());
                std::thread::spawn(move || {
                    node.parent_id = Some(under);
                    barrier.wait();
                    state.repository.move_organization(&node)
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        let moved = results.iter().filter(|r| matches!(r, Ok(scaffoldry_server::repository::OrgMove::Moved))).count();
        let refused = results.iter().filter(|r| matches!(r, Ok(scaffoldry_server::repository::OrgMove::Cycle))).count();
        assert_eq!((moved, refused), (1, 1), "round {round}: one move wins and one is refused: {results:?}");

        let listed = state.repository.list_organizations().unwrap();
        let parent = |id: Uuid| listed.iter().find(|o| o.id == id).and_then(|o| o.parent_id);
        for start in [x.id, y.id] {
            let (mut cur, mut steps) = (Some(start), 0);
            while let Some(c) = cur {
                steps += 1;
                assert!(steps < 10, "round {round}: the tree has a cycle");
                cur = parent(c);
            }
        }
    }
}
