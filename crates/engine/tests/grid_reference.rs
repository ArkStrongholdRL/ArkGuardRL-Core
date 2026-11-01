use arkguard_engine::battle::{
    direction::{Direction, Placement},
    grid::{bresenham_tiles, segment_tiles, Grid, Rect, BLOCK, COLS, CRATE},
};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command, Stdio},
};
fn point(v: &Value) -> [i32; 2] {
    [v[0].as_i64().unwrap() as i32, v[1].as_i64().unwrap() as i32]
}
fn direction(v: &Value) -> Direction {
    Direction::ALL
        .into_iter()
        .find(|d| d.name() == v.as_str().unwrap())
        .unwrap()
}
#[test]
#[ignore = "Requires ARKGUARD_REFERENCE_ROOT, Node and external stage data; no matches"]
fn all_stage_fields_match() {
    let root = std::env::var("ARKGUARD_REFERENCE_ROOT").unwrap();
    let stages: Value =
        serde_json::from_slice(&std::fs::read(Path::new(&root).join("data/stages.json")).unwrap())
            .unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/reference/grid.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(&root)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut grid = None;
    let mut count = 0;
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        match v["kind"].as_str().unwrap() {
            "direction" => {
                let d = direction(&v["d"]);
                let p = [
                    v["r"].as_i64().unwrap() as i32,
                    v["c"].as_i64().unwrap() as i32,
                ];
                assert_eq!(json!(d.rotate(p)), v["rotated"]);
                assert_eq!(json!(d.local(p)), v["local"]);
                assert_eq!(d.mirror().name(), v["mirror"]);
                assert_eq!(d.opposite().name(), v["opposite"]);
                assert_eq!(
                    Direction::from_delta(p[0] as f64, p[1] as f64, d).name(),
                    v["delta"]
                );
            }
            "placement" => {
                let p = Placement {
                    boss_like: v["battle"] != "normal",
                    right_side: v["side"] == "R",
                    field_coords: v["field"].as_bool().unwrap(),
                    row_offset: v["offset"].as_i64().map(|x| x as i32),
                    col_offset: if v["side"] == "R" { 8 } else { 0 },
                };
                let abs = v["abs"].as_bool().unwrap();
                assert_eq!(json!(p.map_tile(point(&v["p"]), abs)), v["tile"]);
                assert_eq!(p.map_dir(Direction::Right, abs).name(), v["dir"]);
            }
            "line" => {
                assert_eq!(
                    json!(bresenham_tiles(point(&v["a"]), point(&v["b"]))),
                    v["bresenham"]
                );
                assert_eq!(
                    json!(segment_tiles(point(&v["a"]), point(&v["b"]))),
                    v["segment"]
                );
            }
            "grid" => {
                let mut stage = stages[v["id"].as_str().unwrap()].clone();
                if !stage["tiles"].is_null() {
                    stage["legend"] = stage["tiles"].clone();
                }
                let r = &v["rect"];
                let mut g = Grid::new(
                    &stage,
                    Rect {
                        r0: r["r0"].as_i64().unwrap() as i32,
                        r1: r["r1"].as_i64().unwrap() as i32,
                        c0: r["c0"].as_i64().unwrap() as i32,
                        c1: r["c1"].as_i64().unwrap() as i32,
                    },
                )
                .unwrap();
                if v["obstacles"] == true {
                    g.set_obstacle([9, 8], true, CRATE).unwrap();
                    g.set_obstacle([10, 8], true, BLOCK).unwrap();
                    g.set_obstacle([9, 8], true, BLOCK).unwrap();
                    g.set_obstacle([9, 8], false, BLOCK).unwrap();
                }
                for (k, t) in g.tiles().iter().enumerate() {
                    assert_eq!(
                        json!([t.glyph, t.key, t.height, t.build, t.pass, t.terrain, t.special]),
                        v["tiles"][k],
                        "stage {} tile {k}",
                        v["id"]
                    );
                    let p = [(k / COLS) as i32, (k % COLS) as i32];
                    assert_eq!(
                        json!([
                            g.ground_passable(p, false),
                            g.walkable(p, false),
                            g.fly_passable(p),
                            g.can_stand(p, false),
                            g.can_stand(p, true)
                        ]),
                        v["queries"][k]
                    );
                }
                assert_eq!(g.version(), v["version"].as_u64().unwrap());
                grid = Some(g);
            }
            "straight" => {
                assert_eq!(
                    grid.as_ref().unwrap().straight_clear(
                        v["x"].as_f64().unwrap(),
                        v["y"].as_f64().unwrap(),
                        point(&v["end"])
                    ),
                    v["value"].as_bool().unwrap()
                );
            }
            "flow" => {
                let f = grid.as_mut().unwrap().flow_field(
                    point(&v["end"]),
                    v["diagonal"].as_bool().unwrap(),
                    v["ignore"].as_bool().unwrap(),
                );
                for (name, values) in [
                    ("dist", &f.dist),
                    ("parent", &f.parent),
                    ("pen", &f.pen),
                    ("next", &f.next),
                    ("official", &f.official),
                    ("cost", &f.cost),
                    ("deep", &f.deep),
                ] {
                    assert_eq!(
                        json!(values.as_slice()),
                        v[name],
                        "stage {} field {name}, rect {}, end {}, diagonal {}, ignore {}",
                        v["id"],
                        v["rect"],
                        v["end"],
                        v["diagonal"],
                        v["ignore"]
                    );
                }
                for path in v["paths"].as_array().unwrap() {
                    let start = point(&path["start"]);
                    let length = Grid::field_length(&f, start);
                    if path["length"].is_null() {
                        assert!(length.is_infinite());
                    } else {
                        assert_eq!(length.to_bits(), path["length"].as_f64().unwrap().to_bits());
                    }
                    let waypoints = grid.as_mut().unwrap().waypoints(
                        start,
                        point(&v["end"]),
                        v["diagonal"].as_bool().unwrap(),
                        v["ignore"].as_bool().unwrap(),
                    );
                    assert_eq!(json!(waypoints), path["waypoints"]);
                }
                count += 1;
            }
            _ => panic!("unknown case"),
        }
    }
    assert!(child.wait().unwrap().success());
    assert!(count > 0);
    eprintln!("Compared {count} complete fields");
}
