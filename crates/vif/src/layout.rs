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
                let (one, _, two) = parts(area, gap, *across, *ratio);
                first.layout(one, gap, out);
                second.layout(two, gap, out);
            }
        }
    }

    fn has(&self, target: PaneId) -> bool {
        match self {
            Node::Pane(id) => *id == target,
            Node::Split { first, second, .. } => first.has(target) || second.has(target),
        }
    }

    /// Moves the divider closest to `target` that lies across `direction` by `step` of the
    /// split it parts; false when no divider lies that way.
    pub fn resize(&mut self, target: PaneId, direction: Direction, step: f32) -> bool {
        let across = matches!(direction, Direction::Left | Direction::Right);
        let delta = if matches!(direction, Direction::Right | Direction::Down) {
            step
        } else {
            -step
        };
        self.nudge(target, across, delta)
    }

    fn nudge(&mut self, target: PaneId, axis: bool, delta: f32) -> bool {
        let Node::Split {
            across,
            ratio,
            first,
            second,
        } = self
        else {
            return false;
        };
        let inner = if first.has(target) {
            first.nudge(target, axis, delta)
        } else if second.has(target) {
            second.nudge(target, axis, delta)
        } else {
            return false;
        };
        if inner || *across != axis {
            return inner;
        }
        *ratio = clamp_ratio(*ratio + delta);
        true
    }

    /// Every divider inside `area`, for the mouse to take hold of.
    pub fn dividers(&self, area: Area, gap: f32, path: &mut Vec<bool>, out: &mut Vec<Seam>) {
        if let Node::Split {
            across,
            ratio,
            first,
            second,
        } = self
        {
            let (one, line, two) = parts(area, gap, *across, *ratio);
            out.push(Seam {
                path: path.clone(),
                across: *across,
                line,
                whole: area,
            });
            path.push(false);
            first.dividers(one, gap, path, out);
            path.pop();
            path.push(true);
            second.dividers(two, gap, path, out);
            path.pop();
        }
    }

    /// Sets the ratio of the split `path` leads to, false taking the first side at each step.
    pub fn set_ratio(&mut self, path: &[bool], to: f32) {
        let Node::Split {
            ratio, first, second, ..
        } = self
        else {
            return;
        };
        match path.split_first() {
            None => *ratio = clamp_ratio(to),
            Some((false, rest)) => first.set_ratio(rest, to),
            Some((true, rest)) => second.set_ratio(rest, to),
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

/// The space between a split's two sides.
#[derive(Clone, Debug, PartialEq)]
pub struct Seam {
    pub path: Vec<bool>,
    pub across: bool,
    pub line: Area,
    /// What the split parts, so a drag knows what share the pointer stands for.
    pub whole: Area,
}

impl Seam {
    /// The ratio that puts the divider under the pointer.
    pub fn ratio_at(&self, x: f32, y: f32, gap: f32) -> f32 {
        let (at, from, length) = if self.across {
            (x, self.whole.x, self.whole.w)
        } else {
            (y, self.whole.y, self.whole.h)
        };
        clamp_ratio((at - from - gap / 2.0) / (length - gap).max(1.0))
    }
}

fn clamp_ratio(ratio: f32) -> f32 {
    ratio.clamp(0.1, 0.9)
}

/// The first side, the gap and the second side of `area` split at `ratio`.
fn parts(area: Area, gap: f32, across: bool, ratio: f32) -> (Area, Area, Area) {
    if across {
        let w = ((area.w - gap) * ratio).round();
        let x = area.x + w + gap;
        (
            Area { w, ..area },
            Area {
                x: area.x + w,
                w: gap,
                ..area
            },
            Area {
                x,
                w: area.x + area.w - x,
                ..area
            },
        )
    } else {
        let h = ((area.h - gap) * ratio).round();
        let y = area.y + h + gap;
        (
            Area { h, ..area },
            Area {
                y: area.y + h,
                h: gap,
                ..area
            },
            Area {
                y,
                h: area.y + area.h - y,
                ..area
            },
        )
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

    #[test]
    fn dividers_move_by_keys_and_by_the_pointer() {
        let area = Area {
            x: 0.0,
            y: 0.0,
            w: 101.0,
            h: 51.0,
        };
        let mut root = Node::Pane(1);
        root.split(1, 2, true);
        root.split(2, 3, false);
        // The pane below moves the divider between 2 and 3, the one on the left the outer one.
        assert!(root.resize(3, Direction::Up, 0.1));
        assert!(root.resize(1, Direction::Right, 0.1));
        assert!(!root.resize(1, Direction::Down, 0.1));
        let Node::Split { ratio, second, .. } = &root else {
            unreachable!()
        };
        assert!((ratio - 0.6).abs() < 1e-6);
        assert!(matches!(**second, Node::Split { ratio, .. } if (ratio - 0.4).abs() < 1e-6));

        let mut out = Vec::new();
        root.dividers(area, 1.0, &mut Vec::new(), &mut out);
        assert_eq!(out.len(), 2);
        assert!(out[0].across && out[0].line.x == 60.0 && out[0].line.w == 1.0);
        assert_eq!(out[1].path, vec![true]);
        root.set_ratio(&out[0].path, out[0].ratio_at(25.5, 10.0, 1.0));
        let mut panes = Vec::new();
        root.layout(area, 1.0, &mut panes);
        assert_eq!(panes[0].1.w, 25.0);
        root.set_ratio(&[], 2.0);
        let Node::Split { ratio, .. } = &root else {
            unreachable!()
        };
        assert_eq!(*ratio, 0.9);
    }
}
