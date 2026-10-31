/// A binary pane tree. Leaf IDs stay stable when adjacent panes close.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Columns,
    Rows,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    Pane(usize),
    Split {
        id: usize,
        axis: Axis,
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}
impl Layout {
    pub fn split(&mut self, target: usize, new_pane: usize, split_id: usize, axis: Axis) -> bool {
        match self {
            Self::Pane(id) if *id == target => {
                *self = Self::Split {
                    id: split_id,
                    axis,
                    ratio: 0.5,
                    first: Box::new(Self::Pane(target)),
                    second: Box::new(Self::Pane(new_pane)),
                };
                true
            }
            Self::Split { first, second, .. } => {
                first.split(target, new_pane, split_id, axis)
                    || second.split(target, new_pane, split_id, axis)
            }
            _ => false,
        }
    }
    /// A root leaf cannot be removed; its tab must close instead.
    pub fn remove(&mut self, target: usize) -> bool {
        match self {
            Self::Split { first, second, .. } if **first == Self::Pane(target) => {
                *self = *second.clone();
                true
            }
            Self::Split { first, second, .. } if **second == Self::Pane(target) => {
                *self = *first.clone();
                true
            }
            Self::Split { first, second, .. } => first.remove(target) || second.remove(target),
            _ => false,
        }
    }
    pub fn set_ratio(&mut self, target: usize, new_ratio: f32) -> bool {
        match self {
            Self::Split { id, ratio, .. } if *id == target => {
                if new_ratio.is_finite() {
                    *ratio = new_ratio.clamp(0.15, 0.85);
                }
                true
            }
            Self::Split { first, second, .. } => {
                first.set_ratio(target, new_ratio) || second.set_ratio(target, new_ratio)
            }
            _ => false,
        }
    }
    pub fn panes(&self) -> Vec<usize> {
        match self {
            Self::Pane(id) => vec![*id],
            Self::Split { first, second, .. } => [first.panes(), second.panes()].concat(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_close_promotes_sibling_without_losing_other_panes() {
        let mut tree = Layout::Pane(1);
        tree.split(1, 2, 10, Axis::Columns);
        tree.split(2, 3, 11, Axis::Rows);
        assert!(tree.remove(2));
        assert_eq!(tree.panes(), vec![1, 3]);
        assert!(tree.remove(1));
        assert_eq!(tree, Layout::Pane(3));
        assert!(!tree.remove(3));
    }
    #[test]
    fn missing_targets_preserve_the_layout_and_ratios_are_bounded() {
        let mut tree = Layout::Pane(1);
        assert!(!tree.split(9, 2, 10, Axis::Columns));
        tree.split(1, 2, 10, Axis::Columns);
        tree.set_ratio(10, 2.0);
        tree.set_ratio(10, f32::NAN);
        if let Layout::Split { ratio, .. } = tree {
            assert_eq!(ratio, 0.85);
        } else {
            panic!("Expected split");
        }
    }
}
