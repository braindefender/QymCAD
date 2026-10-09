//! A LINE IS EXTENDED AS THE EXTEND TOOL ASKS: the nearer end to the nearest curve across its axis, or to the curve the
//! pointer is over; both ends with "Both sides", each to its own; an end with nothing on its side stays.
//!
//! Reported behaviour: Extend stretched the end nearer the click to the nearest crossing at once - no choice of which
//! curve to stop at, and no way to extend both ends.
use qymcad_core::feature::Purpose;
use qymcad_core::geom::Point2;
use qymcad_core::model::{ExtendAsk, ExtendSides, Project};

/// The line (0, 0) - (10, 0) and vertical lines across its axis at each of `across`. Answers the project, the sketch, the
/// line and the vertical lines in the order given.
fn lines(across: &[f64]) -> (Project, usize, u64, Vec<u64>) {
    let mut p = Project::default();
    p.new_document();
    let si = p.new_sketch("S");
    p.add_line_entity(si, 0.0, 0.0, 10.0, 0.0, Purpose::Real);
    for &x in across {
        p.add_line_entity(si, x, -5.0, x, 5.0, Purpose::Real);
    }
    let ids: Vec<u64> = p.sketches[si].entities.iter().map(|e| e.id).collect();
    (p, si, ids[0], ids[1..].to_vec())
}

/// Where the ends of line `eid` stand, x of the start and of the end.
fn ends_x(p: &Project, si: usize, eid: u64) -> (f64, f64) {
    let e = p.sketches[si].entities.iter().find(|e| e.id == eid).expect("the line");
    let qymcad_core::model::EntityKind::Line { a, b } = e.kind else { panic!("not a line") };
    let x = |id| p.sketches[si].points.iter().find(|q| q.id == id).map(|q| q.x).expect("the point");
    (x(a), x(b))
}

/// THE ASK: the pointer at x on the axis, over the vertical line `over` of those made, which ends.
struct Case {
    name: &'static str,
    across: &'static [f64],
    pointer_x: f64,
    over: Option<usize>,
    sides: ExtendSides,
    ends: (f64, f64),
}

#[test]
fn a_line_is_extended_to_where_it_is_asked() {
    let cases = [
        Case { name: "the nearer end to the nearest", across: &[-15.0, 20.0, 30.0], pointer_x: 9.0, over: None, sides: ExtendSides::Nearer, ends: (0.0, 20.0) },
        Case { name: "the start, the pointer nearer it", across: &[-15.0, 20.0, 30.0], pointer_x: 1.0, over: None, sides: ExtendSides::Nearer, ends: (-15.0, 10.0) },
        Case { name: "to the farther line the pointer is over", across: &[-15.0, 20.0, 30.0], pointer_x: 30.0, over: Some(2), sides: ExtendSides::Nearer, ends: (0.0, 30.0) },
        Case { name: "both sides, each to its nearest", across: &[-15.0, 20.0, 30.0], pointer_x: 5.0, over: None, sides: ExtendSides::Both, ends: (-15.0, 20.0) },
        Case { name: "both sides, the far one taken on its side", across: &[-15.0, 20.0, 30.0], pointer_x: 30.0, over: Some(2), sides: ExtendSides::Both, ends: (-15.0, 30.0) },
        Case { name: "both sides, nothing before the start", across: &[20.0, 30.0], pointer_x: 5.0, over: None, sides: ExtendSides::Both, ends: (0.0, 20.0) },
        Case { name: "the start, nothing on its side", across: &[20.0], pointer_x: 1.0, over: None, sides: ExtendSides::Nearer, ends: (0.0, 10.0) },
    ];
    let mut failures = Vec::new();
    for case in cases {
        let (mut p, si, line, across) = lines(case.across);
        let ask = ExtendAsk { pointer: Point2::new(case.pointer_x, 0.0), over: case.over.map(|k| across[k]), sides: case.sides };
        let ext = p.line_extension(si, line, &ask);
        p.extend_line_by(si, line, ext);
        let got = ends_x(&p, si, line);
        if (got.0 - case.ends.0).abs() > 1e-6 || (got.1 - case.ends.1).abs() > 1e-6 {
            failures.push(format!("{}: the line runs {got:?}, wanted {:?}", case.name, case.ends));
        }
    }
    assert!(failures.is_empty(), "a line extended as asked:\n{}", failures.join("\n"));
}
