//! Row/column layout: measures widget boxes and returns each widget's origin.

use super::Align;

/// Layout tree with leaves pointing into the widget list.
#[derive(Debug)]
pub enum LayoutTree {
    Leaf(usize),
    Row(Vec<LayoutTree>, Align),
    Column(Vec<LayoutTree>, Align),
}

/// Origins (indexed like `sizes`) and total size, logical px.
pub fn arrange(tree: &LayoutTree, sizes: &[(f32, f32)], gap: f32) -> (Vec<(f32, f32)>, (f32, f32)) {
    let mut origins = vec![(0.0, 0.0); sizes.len()];
    let total = measure(tree, sizes, gap);
    place(tree, sizes, gap, (0.0, 0.0), &mut origins);
    (origins, total)
}

fn measure(tree: &LayoutTree, sizes: &[(f32, f32)], gap: f32) -> (f32, f32) {
    match tree {
        LayoutTree::Leaf(i) => sizes[*i],
        LayoutTree::Row(children, _) => {
            let (w, h) = children
                .iter()
                .map(|c| measure(c, sizes, gap))
                .fold((0.0, 0.0f32), |(w, h), (cw, ch)| (w + cw, h.max(ch)));
            (w + gap * children.len().saturating_sub(1) as f32, h)
        }
        LayoutTree::Column(children, _) => {
            let (w, h) = children
                .iter()
                .map(|c| measure(c, sizes, gap))
                .fold((0.0f32, 0.0), |(w, h), (cw, ch)| (w.max(cw), h + ch));
            (w, h + gap * children.len().saturating_sub(1) as f32)
        }
    }
}

fn place(
    tree: &LayoutTree,
    sizes: &[(f32, f32)],
    gap: f32,
    at: (f32, f32),
    out: &mut [(f32, f32)],
) {
    match tree {
        LayoutTree::Leaf(i) => out[*i] = at,
        LayoutTree::Row(children, align) => {
            let (_, row_h) = measure(tree, sizes, gap);
            let mut x = at.0;
            for child in children {
                let (cw, ch) = measure(child, sizes, gap);
                place(
                    child,
                    sizes,
                    gap,
                    (x, at.1 + align_offset(*align, row_h - ch)),
                    out,
                );
                x += cw + gap;
            }
        }
        LayoutTree::Column(children, align) => {
            let (col_w, _) = measure(tree, sizes, gap);
            let mut y = at.1;
            for child in children {
                let (cw, ch) = measure(child, sizes, gap);
                place(
                    child,
                    sizes,
                    gap,
                    (at.0 + align_offset(*align, col_w - cw), y),
                    out,
                );
                y += ch + gap;
            }
        }
    }
}

fn align_offset(align: Align, free: f32) -> f32 {
    match align {
        Align::Start => 0.0,
        Align::Center => free / 2.0,
        Align::End => free,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use LayoutTree::{Column, Leaf, Row};

    #[test]
    fn movement_over_keyboard_and_mouse() {
        // movement (0) on top; keyboard (1) : mouse (2) below, bottoms aligned.
        let tree = Column(
            vec![Leaf(0), Row(vec![Leaf(1), Leaf(2)], Align::End)],
            Align::Start,
        );
        let sizes = [(200.0, 100.0), (300.0, 150.0), (100.0, 120.0)];
        let (origins, total) = arrange(&tree, &sizes, 10.0);

        assert_eq!(total, (410.0, 260.0));
        assert_eq!(origins[0], (0.0, 0.0));
        assert_eq!(origins[1], (0.0, 110.0));
        assert_eq!(origins[2], (310.0, 140.0)); // 110 + (150 - 120)
    }

    #[test]
    fn center_align_in_column() {
        let tree = Column(vec![Leaf(0), Leaf(1)], Align::Center);
        let (origins, total) = arrange(&tree, &[(100.0, 10.0), (50.0, 10.0)], 0.0);
        assert_eq!(total, (100.0, 20.0));
        assert_eq!(origins[1], (25.0, 10.0));
    }
}
