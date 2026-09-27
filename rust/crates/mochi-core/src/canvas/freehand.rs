//! 定义自由绘制笔迹、边界和画布文字使用的基础数据及几何计算。
use super::*;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Bounds {
    pub fn contains(self, point: Point, margin: f64) -> bool {
        point.x >= self.left - margin
            && point.x <= self.right + margin
            && point.y >= self.top - margin
            && point.y <= self.bottom + margin
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextNote {
    pub id: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub text: String,
    pub color: u32,
    pub font_size: f64,
}

impl TextNote {
    pub fn bounds(&self) -> Bounds {
        Bounds {
            left: self.x,
            top: self.y,
            right: self.x + self.width,
            bottom: self.y + self.height,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    pub id: String,
    pub points: Vec<Point>,
    pub color: u32,
    pub width: f64,
}

impl Stroke {
    pub fn bounds(&self) -> Option<Bounds> {
        let first = self.points.first()?;
        let mut bounds = Bounds {
            left: first.x,
            top: first.y,
            right: first.x,
            bottom: first.y,
        };
        for point in &self.points {
            bounds.left = bounds.left.min(point.x);
            bounds.top = bounds.top.min(point.y);
            bounds.right = bounds.right.max(point.x);
            bounds.bottom = bounds.bottom.max(point.y);
        }
        let radius = self.width / 2.0;
        Some(Bounds {
            left: bounds.left - radius,
            top: bounds.top - radius,
            right: bounds.right + radius,
            bottom: bounds.bottom + radius,
        })
    }

    pub fn hit(&self, point: Point, tolerance: f64) -> bool {
        if !self.bounds().is_some_and(|b| b.contains(point, tolerance)) {
            return false;
        }
        let radius = tolerance + self.width / 2.0;
        self.points
            .iter()
            .any(|p| (p.x - point.x).hypot(p.y - point.y) <= radius)
            || self
                .points
                .windows(2)
                .any(|pair| segment_distance(point, pair[0], pair[1]) <= radius)
    }
}

fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx * dx + dy * dy;
    if length <= f64::EPSILON {
        return (p.x - a.x).hypot(p.y - a.y);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / length).clamp(0.0, 1.0);
    (p.x - a.x - t * dx).hypot(p.y - a.y - t * dy)
}

impl CanvasDocument {
    pub fn next_id(&self, prefix: &str) -> String {
        let ids: HashSet<&str> = self
            .cards
            .iter()
            .map(|c| c.id.as_str())
            .chain(self.texts.iter().map(|t| t.id.as_str()))
            .chain(self.strokes.iter().map(|s| s.id.as_str()))
            .collect();
        (1..)
            .map(|n| format!("{prefix}-{n:08x}"))
            .find(|id| !ids.contains(id.as_str()))
            .unwrap()
    }

    pub fn bounds(&self) -> Option<Bounds> {
        self.cards
            .iter()
            .map(|c| Bounds {
                left: c.x,
                top: c.y,
                right: c.x + c.width,
                bottom: c.y + c.height,
            })
            .chain(self.texts.iter().map(TextNote::bounds))
            .chain(self.strokes.iter().filter_map(Stroke::bounds))
            .reduce(Bounds::union)
    }

    pub(super) fn validate_freehand(&self) -> Result<()> {
        let mut ids = HashSet::new();
        for id in self
            .cards
            .iter()
            .map(|c| &c.id)
            .chain(self.texts.iter().map(|t| &t.id))
            .chain(self.strokes.iter().map(|s| &s.id))
        {
            if !valid_id(id) || !ids.insert(id) {
                bail!("画布对象 ID 无效或重复");
            }
        }
        for text in &self.texts {
            if !text.x.is_finite()
                || !text.y.is_finite()
                || !(40.0..=10000.0).contains(&text.width)
                || !(24.0..=1_000_000.0).contains(&text.height)
                || !(12.0..=96.0).contains(&text.font_size)
                || text.color > 0xffffff
                || text.text.len() > 1_000_000
            {
                bail!("画布文字格式无效");
            }
        }
        for stroke in &self.strokes {
            if stroke.points.is_empty()
                || stroke.points.len() > 100_000
                || !(0.5..=64.0).contains(&stroke.width)
                || stroke.color > 0xffffff
                || stroke
                    .points
                    .iter()
                    .any(|p| !p.x.is_finite() || !p.y.is_finite())
            {
                bail!("画布笔迹格式无效");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_one_migrates_without_losing_reference_geometry() {
        let raw = r#"{"version":1,"viewport":{"x":-50,"y":100,"zoom":0.5},"cards":[{"id":"card-1","x":-120,"y":60,"target":{"type":"mochi","url":"mochi://open?path=a.md&kind=file"}}]}"#;
        let document = parse(raw).unwrap();
        assert_eq!(document.version, VERSION);
        assert_eq!(document.cards[0].x, -120.0);
        assert!(document.texts.is_empty() && document.strokes.is_empty());
        assert_eq!(parse(&serialize(&document).unwrap()).unwrap(), document);
    }
    #[test]
    fn freehand_round_trip_preserves_multiline_unicode_and_single_point() {
        let mut d = CanvasDocument::empty();
        d.texts.push(TextNote {
            id: "t".into(),
            x: -99999.0,
            y: 5000.0,
            width: 320.0,
            height: 160.0,
            text: "中文😀\n自由笔记".into(),
            color: 0x2563eb,
            font_size: 24.0,
        });
        d.strokes.push(Stroke {
            id: "s".into(),
            points: vec![Point {
                x: -123.0,
                y: 234.0,
            }],
            color: 0x334155,
            width: 4.0,
        });
        assert_eq!(parse(&serialize(&d).unwrap()).unwrap(), d);
        assert!(d.strokes[0].hit(
            Point {
                x: -123.0,
                y: 234.0
            },
            0.0
        ));
    }
    #[test]
    fn rejects_invalid_geometry_and_duplicate_ids() {
        let mut d = CanvasDocument::empty();
        d.viewport.x = f64::NAN;
        assert!(d.validate().is_err());
        d.viewport.x = 0.0;
        let stroke = Stroke {
            id: "s".into(),
            points: vec![],
            color: 0,
            width: 4.0,
        };
        d.strokes.push(stroke);
        assert!(d.validate().is_err());
        d.strokes[0].points.push(Point { x: 1.0, y: 2.0 });
        assert!(d.validate().is_ok());
        d.strokes.push(d.strokes[0].clone());
        assert!(d.validate().is_err());
    }
    #[test]
    fn stroke_hit_tests_segments_rather_than_the_whole_bounding_box() {
        let s = Stroke {
            id: "s".into(),
            points: vec![Point { x: 0.0, y: 0.0 }, Point { x: 100.0, y: 100.0 }],
            color: 0,
            width: 4.0,
        };
        assert!(s.hit(Point { x: 50.0, y: 50.0 }, 2.0));
        assert!(!s.hit(Point { x: 10.0, y: 90.0 }, 2.0));
    }
    #[test]
    fn deleting_then_adding_a_card_never_reuses_an_existing_id() {
        let mut d = CanvasDocument::empty();
        for _ in 0..3 {
            d.add_mochi_reference("mochi://open?path=a.md&kind=file")
                .unwrap();
        }
        d.cards.remove(1);
        d.add_mochi_reference("mochi://open?path=a.md&kind=file")
            .unwrap();
        assert!(d.validate().is_ok());
    }
}
