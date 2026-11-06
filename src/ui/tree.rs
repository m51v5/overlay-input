//! Profile layout tree, addressed by paths of child indexes: `{ row | column: [...], align? }` containers
//! and `{ widget, ...options }` leaves.

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Row,
    Column,
    Widget,
}

impl Kind {
    pub fn key(self) -> &'static str {
        match self {
            Kind::Row => "row",
            Kind::Column => "column",
            Kind::Widget => "widget",
        }
    }
}

pub fn kind(node: &Value) -> Option<Kind> {
    if node["row"].is_array() {
        Some(Kind::Row)
    } else if node["column"].is_array() {
        Some(Kind::Column)
    } else if node["widget"].is_string() {
        Some(Kind::Widget)
    } else {
        None
    }
}

pub fn children(node: &Value) -> Option<&Vec<Value>> {
    match kind(node)? {
        Kind::Widget => None,
        k => node[k.key()].as_array(),
    }
}

pub fn children_mut(node: &mut Value) -> Option<&mut Vec<Value>> {
    match kind(node)? {
        Kind::Widget => None,
        k => node.get_mut(k.key())?.as_array_mut(),
    }
}

pub fn get<'a>(root: &'a Value, path: &[usize]) -> Option<&'a Value> {
    path.iter()
        .try_fold(root, |node, &i| children(node)?.get(i))
}

pub fn get_mut<'a>(root: &'a mut Value, path: &[usize]) -> Option<&'a mut Value> {
    path.iter()
        .try_fold(root, |node, &i| children_mut(node)?.get_mut(i))
}

pub fn new_node(kind: Kind, widget: Option<&str>) -> Value {
    match kind {
        Kind::Widget => json!({ "widget": widget.unwrap_or("keyboard") }),
        k => Value::Object(Map::from_iter([(k.key().to_owned(), json!([]))])),
    }
}

/// Row ↔ column, keeping children and align.
pub fn set_container_kind(node: &mut Value, to: Kind) {
    let (Some(from), Some(map)) = (kind(node), node.as_object_mut()) else {
        return;
    };
    if from == to || from == Kind::Widget || to == Kind::Widget {
        return;
    }
    if let Some(list) = map.remove(from.key()) {
        map.insert(to.key().to_owned(), list);
    }
}

/// Removes the node at `path` (not the root).
pub fn remove(root: &mut Value, path: &[usize]) -> Option<Value> {
    let (&last, parent) = path.split_last()?;
    let list = children_mut(get_mut(root, parent)?)?;
    (last < list.len()).then(|| list.remove(last))
}

/// Inserts `node` into the container at `parent` at `index` (clamped); returns its path.
pub fn insert(root: &mut Value, parent: &[usize], index: usize, node: Value) -> Option<Vec<usize>> {
    let list = children_mut(get_mut(root, parent)?)?;
    let index = index.min(list.len());
    list.insert(index, node);
    Some([parent, &[index]].concat())
}

/// Moves the node at `from` into the container at `parent` at `index`; returns its new path.
/// Refuses moves into the node itself or its own children.
pub fn move_node(
    root: &mut Value,
    from: &[usize],
    parent: &[usize],
    index: usize,
) -> Option<Vec<usize>> {
    if from.is_empty() || parent.starts_with(from) || children(get(root, parent)?).is_none() {
        return None;
    }
    let (&at, from_parent) = from.split_last()?;
    // Removing the node shifts later siblings (and anything under them) down by one.
    let mut parent = parent.to_vec();
    let mut index = index;
    let depth = from_parent.len();
    if parent.len() > depth && parent.starts_with(from_parent) && parent[depth] > at {
        parent[depth] -= 1;
    } else if parent == from_parent && index > at {
        index -= 1;
    }
    let node = remove(root, from)?;
    insert(root, &parent, index, node)
}

/// Number of widgets in the tree.
pub fn widget_count(node: &Value) -> usize {
    match kind(node) {
        Some(Kind::Widget) => 1,
        _ => children(node).map_or(0, |c| c.iter().map(widget_count).sum()),
    }
}

/// True if `path` points into `ancestor` (or is it).
pub fn is_within(path: &[usize], ancestor: &[usize]) -> bool {
    path.starts_with(ancestor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Value {
        json!({ "column": [
            { "widget": "mouse-movement" },
            { "row": [ { "widget": "keyboard" }, { "widget": "mouse" } ], "align": "end" },
        ]})
    }

    fn widget_at(root: &Value, path: &[usize]) -> String {
        get(root, path).unwrap()["widget"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn paths_reach_nodes() {
        let t = tree();
        assert_eq!(kind(&t), Some(Kind::Column));
        assert_eq!(widget_at(&t, &[1, 1]), "mouse");
        assert!(get(&t, &[1, 5]).is_none());
        assert!(get(&t, &[0, 0]).is_none(), "widgets have no children");
        assert_eq!(widget_count(&t), 3);
    }

    #[test]
    fn move_within_one_list_accounts_for_the_removed_slot() {
        let mut t = tree();
        // mouse-movement to the end of the column: index 2 before removal = 1 after.
        let at = move_node(&mut t, &[0], &[], 2).unwrap();
        assert_eq!(at, [1]);
        assert_eq!(widget_at(&t, &[1]), "mouse-movement");
        assert_eq!(kind(get(&t, &[0]).unwrap()), Some(Kind::Row));
    }

    #[test]
    fn move_into_a_later_sibling_container() {
        let mut t = tree();
        // mouse-movement into the row, which shifts from [1] to [0] once it's gone.
        let at = move_node(&mut t, &[0], &[1], 1).unwrap();
        assert_eq!(at, [0, 1]);
        assert_eq!(widget_at(&t, &[0, 0]), "keyboard");
        assert_eq!(widget_at(&t, &[0, 1]), "mouse-movement");
        assert_eq!(widget_at(&t, &[0, 2]), "mouse");
    }

    #[test]
    fn move_out_of_a_container() {
        let mut t = tree();
        let at = move_node(&mut t, &[1, 1], &[], 0).unwrap();
        assert_eq!(at, [0]);
        assert_eq!(widget_at(&t, &[0]), "mouse");
        assert_eq!(children(get(&t, &[2]).unwrap()).unwrap().len(), 1);
    }

    #[test]
    fn no_move_into_itself_or_a_widget() {
        let mut t = tree();
        let before = t.clone();
        assert!(move_node(&mut t, &[1], &[1], 0).is_none());
        assert!(move_node(&mut t, &[1], &[1, 0], 0).is_none());
        assert!(
            move_node(&mut t, &[1, 0], &[0], 0).is_none(),
            "widget is no container"
        );
        assert!(move_node(&mut t, &[], &[1], 0).is_none(), "root stays");
        assert_eq!(t, before);
    }

    #[test]
    fn insert_remove_and_swap_kind() {
        let mut t = tree();
        let at = insert(&mut t, &[1], 9, new_node(Kind::Widget, Some("gamepad"))).unwrap();
        assert_eq!(at, [1, 2], "index clamped to the end");
        assert_eq!(remove(&mut t, &[1, 2]).unwrap()["widget"], "gamepad");
        let row = get_mut(&mut t, &[1]).unwrap();
        set_container_kind(row, Kind::Column);
        assert!(row["column"].is_array() && row.get("row").is_none());
        assert_eq!(row["align"], "end");
        assert!(remove(&mut t, &[]).is_none());
    }
}
