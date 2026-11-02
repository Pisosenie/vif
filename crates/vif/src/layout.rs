pub type PaneId = u64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Area {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }

    fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// A tab's panes: leaves are panes, splits put two subtrees side by side or one over the other.
#[derive(Debug)]
pub enum Node {
    Pane(PaneId),
    Split {
        across: bool,
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    pub fn panes(&self, out: &mut Vec<PaneId>) {
        match self {
            Node::Pane(id) => out.push(*id),
            Node::Split { first, second, .. } => {
                first.panes(out);
                second.panes(out);
            }
        }
    }

    /// Places every pane inside `area`, leaving `gap` pixels between neighbours.
    pub fn layout(&self, area: Area, gap: f32, out: &mut Vec<(PaneId, Area)>) {
        match self {
            Node::Pane(id) => out.push((*id, area)),
            Node::Split {
                across,
                ratio,
                first,
                second,
            } => {
                if *across {
                    let w = ((area.w - gap) * ratio).round();
                    first.layout(Area { w, ..area }, gap, out);
                    let x = area.x + w + gap;
                    second.layout(
                        Area {
                            x,
                            w: area.x + area.w - x,
                            ..area
                        },
                        gap,
                        out,
                    );
                } else {
                    let h = ((area.h - gap) * ratio).round();
                    first.layout(Area { h, ..area }, gap, out);
                    let y = area.y + h + gap;
                    second.layout(
                        Area {
                            y,
                            h: area.y + area.h - y,
                            ..area
                        },
                        gap,
                        out,
                    );
                }
            }
        }
    }

    pub fn split(&mut self, target: PaneId, new: PaneId, across: bool) -> bool {
        match self {
            Node::Pane(id) if *id == target => {
                let old = std::mem::replace(self, Node::Pane(target));
                *self = Node::Split {
                    across,
                    ratio: 0.5,
                    first: Box::new(old),
                    second: Box::new(Node::Pane(new)),
                };
                true
            }
            Node::Pane(_) => false,
            Node::Split { first, second, .. } => first.split(target, new, across) || second.split(target, new, across),
        }
    }

    /// Removes a pane; returns false when it was the last one.
    pub fn remove(&mut self, target: PaneId) -> bool {
        match self {
            Node::Pane(id) => *id != target,
            Node::Split { first, second, .. } => {
                let keep = if matches!(**first, Node::Pane(id) if id == target) {
                    Some(std::mem::replace(&mut **second, Node::Pane(0)))
                } else if matches!(**second, Node::Pane(id) if id == target) {
                    Some(std::mem::replace(&mut **first, Node::Pane(0)))
                } else {
                    None
                };
                match keep {
                    Some(node) => {
                        *self = node;
                        true
                    }
                    None => {
                        first.remove(target);
                        second.remove(target);
                        true
                    }
                }
            }
        }
    }
}

/// The pane whose center lies nearest in `direction` from `from`.
pub fn neighbor(areas: &[(PaneId, Area)], from: PaneId, direction: Direction) -> Option<PaneId> {
    let (_, origin) = areas.iter().find(|(id, _)| *id == from)?;
    let (ox, oy) = origin.center();
    areas
        .iter()
        .filter(|(id, _)| *id != from)
        .filter_map(|(id, a)| {
            let (x, y) = a.center();
            let (along, aside) = match direction {
                Direction::Left => (ox - x, (y - oy).abs()),
                Direction::Right => (x - ox, (y - oy).abs()),
                Direction::Up => (oy - y, (x - ox).abs()),
                Direction::Down => (y - oy, (x - ox).abs()),
            };
            (along > 1.0).then_some((*id, along + aside * 2.0))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_layout_and_remove() {
        let mut root = Node::Pane(1);
        assert!(root.split(1, 2, true));
        assert!(root.split(2, 3, false));
        let mut out = Vec::new();
        root.layout(
            Area {
                x: 0.0,
                y: 0.0,
                w: 101.0,
                h: 51.0,
            },
            1.0,
            &mut out,
        );
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].1.w, 50.0);
        assert_eq!(out[1].1.x, 51.0);
        assert_eq!(neighbor(&out, 1, Direction::Right), Some(2));
        assert_eq!(neighbor(&out, 2, Direction::Down), Some(3));
        assert!(root.remove(2));
        let mut ids = Vec::new();
        root.panes(&mut ids);
        assert_eq!(ids, vec![1, 3]);
        assert!(root.remove(3));
        assert!(!root.remove(1));
    }
}
