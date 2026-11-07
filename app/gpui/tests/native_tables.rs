//! Actual Objective-C adapter selectors, on main, without windows or AX clients.
use gpui::accesskit::{Node, NodeId, Role, Tree, TreeId, TreeUpdate};

fn main() {
    for grouped in [false, true] {
        let mut table = Node::new(Role::Table);
        table.set_label("Synthetic table");
        table.set_column_count(2);
        table.set_row_count(2);
        table.set_children(if grouped {
            vec![NodeId(9)]
        } else {
            vec![NodeId(2), NodeId(5)]
        });
        let mut nodes = vec![(NodeId(1), table)];
        if grouped {
            let mut group = Node::new(Role::RowGroup);
            group.set_children([NodeId(2), NodeId(5)]);
            nodes.push((NodeId(9), group));
        }
        for row_index in 0..2 {
            let row_id = 2 + row_index * 3;
            let mut row = Node::new(Role::Row);
            row.set_row_index(row_index);
            row.set_children([NodeId(row_id as u64 + 1), NodeId(row_id as u64 + 2)]);
            nodes.push((NodeId(row_id as u64), row));
            for column in 0..2 {
                let mut cell = Node::new(if row_index == 0 {
                    Role::ColumnHeader
                } else {
                    Role::Cell
                });
                cell.set_column_index(column);
                cell.set_label(format!("Synthetic column {column}"));
                cell.set_value("Synthetic value");
                nodes.push((NodeId((row_id + column + 1) as u64), cell));
            }
        }
        accesskit_macos::test_native_table(
            TreeUpdate {
                nodes,
                tree: Some(Tree::new(NodeId(1))),
                tree_id: TreeId::ROOT,
                focus: NodeId(1),
            },
            NodeId(1),
        );
    }
    println!(
        "PASS native AXRows/AXColumns/counts/headers/cell indices; flat and grouped synthetic tables"
    );
}
