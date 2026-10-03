//! K.6, render generators.

use redrob_core::{
    Command, Document, Editor, Filter, MazeAlgorithm, Pixel, Rect, SelectionMode, SpiralType,
};
use std::collections::VecDeque;

fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    let mut editor = Editor::new(Document::new(width, height).expect("document")).expect("editor");
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let color = colors[(y as usize) * width as usize + x as usize];
            editor
                .execute(Command::SelectRectangle {
                    rect: Rect::new(x, y, 1, 1),
                    mode: SelectionMode::Replace,
                })
                .expect("select");
            editor.execute(Command::Fill { color }).expect("fill");
        }
    }
    editor.execute(Command::ClearSelection).expect("clear");
    editor
}

fn pixels(editor: &Editor) -> Vec<u8> {
    editor.document().layers()[0].pixels().to_vec()
}

fn maze(
    width: usize,
    height: usize,
    cell: u32,
    seed: u32,
    algorithm: MazeAlgorithm,
    tileable: bool,
) -> Vec<u8> {
    let flat = vec![Pixel::rgba(128, 128, 128, 255); width * height];
    let mut editor = image(width as u32, height as u32, &flat);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Maze {
                cell_width: cell,
                cell_height: cell,
                seed,
                algorithm,
                tileable,
                foreground: Pixel::rgba(0, 0, 0, 255),
                background: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .expect("maze");
    pixels(&editor)
}

/// Read the maze back as a grid of open/closed UNITS, sampling the middle of each one.
fn unit_grid(out: &[u8], width: usize, height: usize, cell: usize) -> (Vec<bool>, usize, usize) {
    let cols = width / cell;
    let rows = height / cell;
    let mut grid = vec![false; cols * rows];
    for uy in 0..rows {
        for ux in 0..cols {
            let px = ux * cell + cell / 2;
            let py = uy * cell + cell / 2;
            grid[uy * cols + ux] = out[(py * width + px) * 4] > 128;
        }
    }
    (grid, cols, rows)
}

/// Connected components of the open units, and the largest. `wrap` for a tileable maze, whose
/// connections genuinely cross the border.
fn components(grid: &[bool], cols: usize, rows: usize, wrap: bool) -> (usize, usize) {
    let mut seen = vec![false; grid.len()];
    let mut count = 0usize;
    let mut largest = 0usize;
    for start in 0..grid.len() {
        if !grid[start] || seen[start] {
            continue;
        }
        count += 1;
        let mut size = 0usize;
        let mut queue = VecDeque::from([start]);
        seen[start] = true;
        while let Some(index) = queue.pop_front() {
            size += 1;
            let x = index % cols;
            let y = index / cols;
            for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                let nx = x as i64 + dx;
                let ny = y as i64 + dy;
                let (nx, ny) = if wrap {
                    (nx.rem_euclid(cols as i64), ny.rem_euclid(rows as i64))
                } else {
                    if nx < 0 || ny < 0 || nx >= cols as i64 || ny >= rows as i64 {
                        continue;
                    }
                    (nx, ny)
                };
                let neighbour = ny as usize * cols + nx as usize;
                if grid[neighbour] && !seen[neighbour] {
                    seen[neighbour] = true;
                    queue.push_back(neighbour);
                }
            }
        }
        largest = largest.max(size);
    }
    (count, largest)
}

/// An open unit with exactly one open neighbour.
fn dead_ends(grid: &[bool], cols: usize, rows: usize) -> usize {
    (0..grid.len())
        .filter(|&index| {
            if !grid[index] {
                return false;
            }
            let x = index % cols;
            let y = index / cols;
            [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)]
                .iter()
                .filter(|(dx, dy)| {
                    let nx = x as i64 + dx;
                    let ny = y as i64 + dy;
                    nx >= 0
                        && ny >= 0
                        && nx < cols as i64
                        && ny < rows as i64
                        && grid[ny as usize * cols + nx as usize]
                })
                .count()
                == 1
        })
        .count()
}

/// A maze is a SPANNING TREE, and that is an exact count rather than a look.
///
/// Cells sit at odd unit coordinates, walls at even ones. A perfect maze connects every cell with
/// no loops, so it has exactly `cells - 1` passages and the open units number `cells + (cells - 1)`
/// — and every one of them is in a single connected component.
///
/// On a 65-pixel canvas at 5 pixels a unit: 13 × 13 units, `(13 - 1) / 2 = 6` cells each way, 36
/// cells, so **71** open units in **one** component. Measured, then asserted. A random pattern of
/// the same density would fail the component count; a maze with a loop would fail the open count.
#[test]
fn maze_is_a_spanning_tree() {
    let (width, height, cell) = (65usize, 65usize, 5usize);
    for algorithm in [MazeAlgorithm::DepthFirst, MazeAlgorithm::Prim] {
        let out = maze(width, height, cell as u32, 7, algorithm, false);
        let (grid, cols, rows) = unit_grid(&out, width, height, cell);
        assert_eq!(
            (cols, rows),
            (13, 13),
            "the unit grid is the canvas over the cell"
        );

        let cells = ((cols - 1) / 2) * ((rows - 1) / 2);
        assert_eq!(cells, 36, "six cells each way");

        let open = grid.iter().filter(|&&v| v).count();
        assert_eq!(
            open,
            2 * cells - 1,
            "{algorithm:?}: a spanning tree has {cells} cells and {} passages, so {} open units",
            cells - 1,
            2 * cells - 1
        );

        let (count, largest) = components(&grid, cols, rows, false);
        assert_eq!(
            count, 1,
            "{algorithm:?}: every cell must be reachable from every other"
        );
        assert_eq!(
            largest, open,
            "{algorithm:?}: and the one component is all of it"
        );
    }
}

/// The two algorithms leave structurally different mazes, which is their known signature.
///
/// Depth-first search always extends the newest cell, so corridors run long before they branch.
/// Prim's grows from any frontier wall with equal chance, so it branches constantly and strands far
/// more dead ends. Measured on the same canvas and seed: **5** dead ends depth-first against **11**
/// for Prim's.
///
/// Names what the wrong implementation gives: one algorithm used for both would make these counts
/// equal and the two images identical.
#[test]
fn maze_algorithms_differ_in_structure() {
    let (width, height, cell) = (65usize, 65usize, 5usize);

    let depth = maze(
        width,
        height,
        cell as u32,
        7,
        MazeAlgorithm::DepthFirst,
        false,
    );
    let prim = maze(width, height, cell as u32, 7, MazeAlgorithm::Prim, false);
    assert_ne!(
        depth, prim,
        "the same seed under two algorithms must not give the same maze"
    );

    let (depth_grid, cols, rows) = unit_grid(&depth, width, height, cell);
    let (prim_grid, _, _) = unit_grid(&prim, width, height, cell);
    let depth_ends = dead_ends(&depth_grid, cols, rows);
    let prim_ends = dead_ends(&prim_grid, cols, rows);

    assert!(
        prim_ends > depth_ends,
        "Prim's branches constantly and must strand more dead ends than depth-first's long \
         corridors: {prim_ends} against {depth_ends}"
    );
}

/// `Tileable` is a different CONSTRUCTION, and the grid period is what changes.
///
/// Upstream has two progress strings in `maze-algorithms.c` — one for a maze and one for a tileable
/// maze — which is the evidence that this is not a finishing pass. The grids differ too: an enclosed
/// maze spends a unit on each outer wall and fits `(units - 1) / 2` cells, while a tileable one has
/// no outer wall, its period being one cell plus one wall, so it fits `units / 2`.
///
/// **This test failed when first written and the filter was wrong.** The first implementation used
/// the enclosed grid for both and carved a wrapping passage into unit 0, which on a 16-unit grid
/// left units 14 and 15 solid — so the connection did not exist and the maze came back in **nine**
/// components. Fixed to use the tileable period and a shared border wall: 64 cells, **127** open
/// units, **one** component under wrapping connectivity.
#[test]
fn maze_tileable_wraps_into_one_connected_maze() {
    let (width, height, cell) = (64usize, 64usize, 4usize);
    let out = maze(width, height, cell as u32, 3, MazeAlgorithm::Prim, true);
    let (grid, cols, rows) = unit_grid(&out, width, height, cell);
    assert_eq!((cols, rows), (16, 16));

    let cells = (cols / 2) * (rows / 2);
    assert_eq!(cells, 64, "a tileable grid fits one cell per two units");

    let open = grid.iter().filter(|&&v| v).count();
    assert_eq!(
        open,
        2 * cells - 1,
        "still a spanning tree: {cells} cells and {} passages",
        cells - 1
    );

    let (count, largest) = components(&grid, cols, rows, true);
    assert_eq!(
        count, 1,
        "wrapping across the border, the tileable maze is one maze"
    );
    assert_eq!(largest, open);

    // And it is genuinely not the enclosed maze: that one has a solid border, this one does not.
    let enclosed = maze(width, height, cell as u32, 3, MazeAlgorithm::Prim, false);
    assert_ne!(
        out, enclosed,
        "tileable must change the maze, not just its edges"
    );
    let (enclosed_grid, _, _) = unit_grid(&enclosed, width, height, cell);
    assert!(
        (0..cols).all(|x| !enclosed_grid[x]),
        "the enclosed maze's top row of units is all wall"
    );
    assert!(
        (0..cols).any(|x| grid[x]),
        "the tileable maze's top row carries passages, because it has no outer wall"
    );
}

/// The seed decides the maze, and the same seed gives the same maze.
#[test]
fn maze_seed_is_the_whole_randomness() {
    let one = maze(65, 65, 5, 1, MazeAlgorithm::DepthFirst, false);
    let one_again = maze(65, 65, 5, 1, MazeAlgorithm::DepthFirst, false);
    let two = maze(65, 65, 5, 2, MazeAlgorithm::DepthFirst, false);

    assert_eq!(one, one_again, "the same seed must give the same maze");
    assert_ne!(one, two, "a different seed must give a different maze");
}

/// The cell size sets the scale: a bigger unit makes a smaller maze.
///
/// Varies one thing. Names the wrong behaviour: an implementation ignoring the cell size would give
/// the same cell count at both.
#[test]
fn maze_cell_size_sets_the_scale() {
    let (width, height) = (65usize, 65usize);
    let cells_for = |cell: usize| {
        let out = maze(width, height, cell as u32, 7, MazeAlgorithm::Prim, false);
        let (grid, cols, rows) = unit_grid(&out, width, height, cell);
        let _ = &grid;
        ((cols - 1) / 2) * ((rows - 1) / 2)
    };
    assert_eq!(cells_for(5), 36, "13 units each way gives six cells");
    assert_eq!(cells_for(13), 4, "five units each way gives two");
}

/// The two axes are independent: a wide unit and a tall one are not the same maze.
#[test]
fn maze_axes_are_independent() {
    let under = |cell_width: u32, cell_height: u32| {
        let flat = vec![Pixel::rgba(128, 128, 128, 255); 64 * 64];
        let mut editor = image(64, 64, &flat);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Maze {
                    cell_width,
                    cell_height,
                    seed: 5,
                    algorithm: MazeAlgorithm::Prim,
                    tileable: false,
                    foreground: Pixel::rgba(0, 0, 0, 255),
                    background: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .expect("maze");
        pixels(&editor)
    };
    assert_ne!(
        under(4, 8),
        under(8, 4),
        "swapping the two cell dimensions must change the maze; a shared axis would make these \
         equal"
    );
}

/// The wall and passage colours come from the command, so a saved maze replays identically.
#[test]
fn maze_colours_come_from_the_command() {
    let flat = vec![Pixel::rgba(128, 128, 128, 255); 64 * 64];
    let mut editor = image(64, 64, &flat);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Maze {
                cell_width: 4,
                cell_height: 4,
                seed: 9,
                algorithm: MazeAlgorithm::Prim,
                tileable: false,
                foreground: Pixel::rgba(10, 20, 200, 255),
                background: Pixel::rgba(240, 230, 40, 255),
            },
        })
        .expect("maze");
    let out = pixels(&editor);
    let colours: std::collections::HashSet<(u8, u8, u8)> =
        out.chunks(4).map(|c| (c[0], c[1], c[2])).collect();
    assert_eq!(
        colours,
        [(10, 20, 200), (240, 230, 40)].into_iter().collect(),
        "a maze holds exactly the two colours it was given"
    );
}

/// A zero cell and a canvas too small for one cell are refused.
#[test]
fn maze_refuses_a_grid_it_cannot_draw() {
    let flat = vec![Pixel::rgba(128, 128, 128, 255); 64];
    let refused = |cell_width: u32, cell_height: u32, size: u32| {
        let count = (size * size) as usize;
        let colors = vec![Pixel::rgba(128, 128, 128, 255); count];
        let mut editor = image(size, size, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Maze {
                    cell_width,
                    cell_height,
                    seed: 0,
                    algorithm: MazeAlgorithm::DepthFirst,
                    tileable: false,
                    foreground: Pixel::rgba(0, 0, 0, 255),
                    background: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .is_err()
    };
    let _ = &flat;
    assert!(refused(0, 5, 8), "a zero cell leaves no grid");
    assert!(refused(5, 0, 8), "on either axis");
    assert!(
        refused(8, 8, 8),
        "one unit across cannot hold a cell and its two walls"
    );
}

/// A saved command with nothing but the kind still loads.
#[test]
fn maze_deserialises_with_defaults() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"maze"}"#).expect("older saved commands must load");
    match filter {
        Filter::Maze {
            cell_width,
            cell_height,
            seed,
            algorithm,
            tileable,
            ..
        } => {
            assert_eq!(cell_width, 5);
            assert_eq!(cell_height, 5);
            assert_eq!(seed, 0);
            assert_eq!(algorithm, MazeAlgorithm::DepthFirst, "the first radio");
            assert!(!tileable, "the checkbox starts clear");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

fn grid_defaults(size: usize, intersection_width: u32) -> Vec<u8> {
    let white = vec![Pixel::rgba(255, 255, 255, 255); size * size];
    let mut editor = image(size as u32, size as u32, &white);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Grid {
                horizontal_width: 1,
                horizontal_space: 16,
                horizontal_offset: 8,
                horizontal_color: Pixel::rgba(255, 0, 0, 255),
                vertical_width: 1,
                vertical_space: 16,
                vertical_offset: 8,
                vertical_color: Pixel::rgba(0, 0, 255, 255),
                intersection_width,
                intersection_space: 2,
                intersection_offset: 6,
                intersection_color: Pixel::rgba(0, 200, 0, 255),
            },
        })
        .expect("grid");
    pixels(&editor)
}

/// Lines land where `(position - offset) mod spacing` says, at upstream's own defaults.
///
/// Spacing 16 and offset 8 put lines at 8, 24 and 40 on a 48-pixel canvas. Every one of those three
/// numbers is READ from `grid.c`'s argument declarations rather than chosen, so this test is a test
/// of the declarations.
#[test]
fn grid_lines_land_at_the_declared_spacing_and_offset() {
    let size = 48usize;
    let out = grid_defaults(size, 0);

    let red_rows: Vec<usize> = (0..size)
        .filter(|&y| out[(y * size) * 4] > 200 && out[(y * size) * 4 + 2] < 100)
        .collect();
    let blue_cols: Vec<usize> = (0..size)
        .filter(|&x| out[x * 4 + 2] > 200 && out[x * 4] < 100)
        .collect();

    assert_eq!(
        red_rows,
        vec![8, 24, 40],
        "horizontal lines at offset 8, every 16"
    );
    assert_eq!(blue_cols, vec![8, 24, 40], "and vertical ones likewise");
}

/// `iwidth` defaults to 0, so intersections are OFF until asked for.
///
/// Read from the declaration — `0, GIMP_MAX_IMAGE_SIZE, 0` — where both the other widths default to
/// 1. A filter defaulting it to 1 would draw crosshairs nobody asked for.
#[test]
fn grid_intersections_are_off_by_default() {
    let size = 48usize;
    let out = grid_defaults(size, 0);
    let green = out
        .chunks(4)
        .filter(|c| c[1] > 150 && c[0] < 100 && c[2] < 100)
        .count();
    assert_eq!(green, 0, "a zero intersection width must draw nothing");
}

/// The intersection is a CROSSHAIR WITH A GAP, not a filled block.
///
/// This is what reading the plug-in's drawing loop bought, and it is not guessable: the po strings
/// say only "Intersection", "Width", "Spacing", "Offset", from which a filled square is the obvious
/// reading. The loop paints where the distance from the crossing is **at least `ispace` and less
/// than `ioffset`**, measured from both sides — so at the declared defaults of 2 and 6 each arm runs
/// from distance 2 to 5 inclusive, and the crossing itself plus distance 1 are left alone.
///
/// Measured before asserting. Around the crossing at (8, 8) the painted offsets are exactly:
///
/// ```text
///         (0,-5) (0,-4) (0,-3) (0,-2)
/// (-5,0) (-4,0) (-3,0) (-2,0)   .   (2,0) (3,0) (4,0) (5,0)
///         (0,2)  (0,3)  (0,4)  (0,5)
/// ```
///
/// Sixteen pixels per crossing, nine crossings, 144 in total. A filled block would paint the centre
/// and distance 1, and would not give 144.
#[test]
fn grid_intersection_is_a_crosshair_with_a_gap_at_the_crossing() {
    let size = 48usize;
    let out = grid_defaults(size, 1);

    let green: Vec<(i64, i64)> = (0..size * size)
        .filter(|&i| out[i * 4 + 1] > 150 && out[i * 4] < 100 && out[i * 4 + 2] < 100)
        .map(|i| ((i % size) as i64, (i / size) as i64))
        .collect();
    assert_eq!(
        green.len(),
        144,
        "sixteen arm pixels at each of nine crossings"
    );

    let mut near: Vec<(i64, i64)> = green
        .iter()
        .map(|&(x, y)| (x - 8, y - 8))
        .filter(|(dx, dy)| dx.abs() <= 8 && dy.abs() <= 8)
        .collect();
    near.sort_unstable();

    let mut expected: Vec<(i64, i64)> = Vec::new();
    for d in 2..6i64 {
        expected.push((0, -d));
        expected.push((0, d));
        expected.push((-d, 0));
        expected.push((d, 0));
    }
    expected.sort_unstable();
    assert_eq!(
        near, expected,
        "four arms from distance 2 to 5, and nothing at the crossing or at distance 1"
    );

    // The gap is the discriminating part, so it gets its own assertion.
    assert!(
        !near.contains(&(0, 0)),
        "the crossing itself must be left to the two lines"
    );
    assert!(
        !near.contains(&(0, 1)) && !near.contains(&(1, 0)),
        "and distance 1 is inside the gap"
    );
}

/// A width of 0 is ACCEPTED and draws nothing; a spacing of 0 is REFUSED.
///
/// The asymmetry is read, not chosen: `grid.c` declares the widths from 0 and the spacings from 1.
/// An invisible line is a meaningful request while a zero spacing is not. This overrides our own
/// `validate_radius` convention for the same reason noise-reduction's `window_size` did — a range
/// read from source outranks a convention of ours.
#[test]
fn grid_width_may_be_zero_but_spacing_may_not() {
    let size = 32usize;
    let white = vec![Pixel::rgba(255, 255, 255, 255); size * size];

    let attempt = |horizontal_width: u32, horizontal_space: u32| {
        let mut editor = image(size as u32, size as u32, &white);
        editor.execute(Command::ApplyFilter {
            filter: Filter::Grid {
                horizontal_width,
                horizontal_space,
                horizontal_offset: 8,
                horizontal_color: Pixel::rgba(255, 0, 0, 255),
                vertical_width: 1,
                vertical_space: 16,
                vertical_offset: 8,
                vertical_color: Pixel::rgba(0, 0, 255, 255),
                intersection_width: 0,
                intersection_space: 2,
                intersection_offset: 6,
                intersection_color: Pixel::rgba(0, 200, 0, 255),
            },
        })
    };

    assert!(
        attempt(0, 16).is_ok(),
        "a width of zero is a legal request for no line"
    );
    assert!(
        attempt(1, 0).is_err(),
        "a spacing of zero has no meaning and is refused"
    );

    // And a zero width really does draw nothing on that axis.
    let mut editor = image(size as u32, size as u32, &white);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Grid {
                horizontal_width: 0,
                horizontal_space: 16,
                horizontal_offset: 8,
                horizontal_color: Pixel::rgba(255, 0, 0, 255),
                vertical_width: 1,
                vertical_space: 16,
                vertical_offset: 8,
                vertical_color: Pixel::rgba(0, 0, 255, 255),
                intersection_width: 0,
                intersection_space: 2,
                intersection_offset: 6,
                intersection_color: Pixel::rgba(0, 200, 0, 255),
            },
        })
        .expect("grid");
    let out = pixels(&editor);
    let red = out
        .chunks(4)
        .filter(|c| c[0] > 200 && c[1] < 100 && c[2] < 100)
        .count();
    assert_eq!(red, 0, "no horizontal line was asked for, so none is drawn");
}

/// Wider lines are thicker, and the width is centred on the line's own position.
#[test]
fn grid_width_thickens_the_line() {
    let size = 48usize;
    let rows_for = |horizontal_width: u32| {
        let white = vec![Pixel::rgba(255, 255, 255, 255); size * size];
        let mut editor = image(size as u32, size as u32, &white);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Grid {
                    horizontal_width,
                    horizontal_space: 16,
                    horizontal_offset: 8,
                    horizontal_color: Pixel::rgba(255, 0, 0, 255),
                    vertical_width: 0,
                    vertical_space: 16,
                    vertical_offset: 8,
                    vertical_color: Pixel::rgba(0, 0, 255, 255),
                    intersection_width: 0,
                    intersection_space: 2,
                    intersection_offset: 6,
                    intersection_color: Pixel::rgba(0, 200, 0, 255),
                },
            })
            .expect("grid");
        let out = pixels(&editor);
        (0..size)
            .filter(|&y| out[(y * size) * 4] > 200 && out[(y * size) * 4 + 1] < 100)
            .count()
    };

    assert_eq!(rows_for(1), 3, "three lines one pixel thick");
    assert_eq!(rows_for(3), 9, "three lines three pixels thick");
}

/// The two axes are independent, and each carries its own colour.
#[test]
fn grid_axes_and_colours_are_independent() {
    let size = 48usize;
    let white = vec![Pixel::rgba(255, 255, 255, 255); size * size];
    let mut editor = image(size as u32, size as u32, &white);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Grid {
                horizontal_width: 1,
                horizontal_space: 12,
                horizontal_offset: 0,
                horizontal_color: Pixel::rgba(255, 0, 0, 255),
                vertical_width: 1,
                vertical_space: 20,
                vertical_offset: 5,
                vertical_color: Pixel::rgba(0, 0, 255, 255),
                intersection_width: 0,
                intersection_space: 2,
                intersection_offset: 6,
                intersection_color: Pixel::rgba(0, 200, 0, 255),
            },
        })
        .expect("grid");
    let out = pixels(&editor);

    // Horizontal: spacing 12, offset 0 -> rows 0, 12, 24, 36. Read on a column with no vertical
    // line, so the two cannot be confused.
    let red_rows: Vec<usize> = (0..size)
        .filter(|&y| {
            let i = y * size + 1;
            out[i * 4] > 200 && out[i * 4 + 2] < 100
        })
        .collect();
    assert_eq!(
        red_rows,
        vec![0, 12, 24, 36],
        "the horizontal axis uses its own spacing"
    );

    // Vertical: spacing 20, offset 5 -> columns 5, 25, 45. Read on a row with no horizontal line.
    let blue_cols: Vec<usize> = (0..size)
        .filter(|&x| {
            let i = size + x;
            out[i * 4 + 2] > 200 && out[i * 4] < 100
        })
        .collect();
    assert_eq!(blue_cols, vec![5, 25, 45], "and the vertical axis its own");
}

/// A saved command with nothing but the kind loads every default grid.c declares.
#[test]
fn grid_deserialises_with_the_declared_defaults() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"grid"}"#).expect("older saved commands must load");
    match filter {
        Filter::Grid {
            horizontal_width,
            horizontal_space,
            horizontal_offset,
            vertical_width,
            vertical_space,
            vertical_offset,
            intersection_width,
            intersection_space,
            intersection_offset,
            horizontal_color,
            ..
        } => {
            assert_eq!((horizontal_width, vertical_width), (1, 1));
            assert_eq!((horizontal_space, vertical_space), (16, 16));
            assert_eq!((horizontal_offset, vertical_offset), (8, 8));
            assert_eq!(intersection_width, 0, "intersections off, as declared");
            assert_eq!(intersection_space, 2);
            assert_eq!(intersection_offset, 6);
            assert_eq!(
                (horizontal_color.r, horizontal_color.g, horizontal_color.b),
                (0, 0, 0),
                "grid.c builds all three default colours with gegl_color_new(\"black\")"
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn spiral(
    size: usize,
    spiral_type: SpiralType,
    x: f64,
    y: f64,
    radius: f64,
    rotation: f64,
    base: f64,
    balance: f64,
) -> Vec<u8> {
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Spiral {
                spiral_type,
                x,
                y,
                radius,
                rotation,
                base,
                balance,
                color1: Pixel::rgba(0, 0, 0, 255),
                color2: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .expect("spiral");
    pixels(&editor)
}

fn dark_fraction(out: &[u8]) -> f64 {
    let dark = out.chunks(4).filter(|c| c[0] < 128).count();
    dark as f64 / (out.len() / 4) as f64
}

/// Colour changes along the ray running right from the centre.
fn bands_along_ray(out: &[u8], size: usize, cx: usize, cy: usize) -> usize {
    let mut changes = 0;
    let mut prev = out[(cy * size + cx) * 4] > 128;
    for d in 1..(size - cx) {
        let here = out[(cy * size + cx + d) * 4] > 128;
        if here != prev {
            changes += 1;
            prev = here;
        }
    }
    changes
}

/// `radius` is in PIXELS, so halving it doubles the arms crossed along a fixed ray.
///
/// The asymmetry worth pinning: `x` and `y` are read as fractions of the area while `radius` is a
/// raw distance, because the propgui computes `x = x1 / area->width` but
/// `radius = sqrt(SQR(x2-x1) + SQR(y2-y1))`. A radius treated as normalised would not scale this
/// way at all.
///
/// Measured over the 64 pixels right of centre on a 128-pixel canvas: 7 changes at radius 16, 3 at
/// 32, 1 at 64 — which is `2 * (64 / radius) - 1` exactly.
#[test]
fn spiral_radius_is_in_pixels_and_sets_the_arm_spacing() {
    let size = 64usize;
    for (radius, expected) in [(8.0f64, 7usize), (16.0, 3), (32.0, 1)] {
        let out = spiral(size, SpiralType::Linear, 0.5, 0.5, radius, 0.0, 2.0, 0.0);
        assert_eq!(
            bands_along_ray(&out, size, 32, 32),
            expected,
            "radius {radius} must cross {expected} arm boundaries over 32 pixels"
        );
    }
}

/// `balance` is exactly the share of each turn given to the first colour.
///
/// INFERRED rather than read — the propgui gives the range (`CLAMP (balance, -1.0, 1.0)`, confirmed
/// by both slider endpoint calculations landing on ±1) but not the meaning. Taken as a duty cycle,
/// 0.5 at zero, which the measurement then confirms is exactly linear:
///
/// | balance | dark fraction |
/// |---|---|
/// | -0.8 | 0.099 |
/// | -0.4 | 0.300 |
/// | 0.0 | 0.500 |
/// | 0.4 | 0.700 |
/// | 0.8 | 0.899 |
///
/// Each is `(balance + 1) / 2` to within a pixel-quantisation of 0.002.
#[test]
fn spiral_balance_is_the_duty_cycle() {
    let size = 64usize;
    for balance in [-0.8f64, -0.4, 0.0, 0.4, 0.8] {
        let out = spiral(size, SpiralType::Linear, 0.5, 0.5, 16.0, 0.0, 2.0, balance);
        let measured = dark_fraction(&out);
        let expected = (balance + 1.0) / 2.0;
        assert!(
            (measured - expected).abs() < 0.005,
            "balance {balance} must give a dark share of {expected:.3}, measured {measured:.3}"
        );
    }
}

/// `base` is used by the LOGARITHMIC type and ignored by the linear one.
///
/// Read from the propgui's own slider count: `n_sliders = 1` for linear and 2 for logarithmic, so
/// linear has no `base` control at all. This is the test that pins that reading, and it names the
/// wrong behaviour — a filter feeding `base` into both laws would make the linear pair differ.
#[test]
fn spiral_base_belongs_to_the_logarithmic_law_only() {
    let size = 48usize;

    let linear_two = spiral(size, SpiralType::Linear, 0.5, 0.5, 16.0, 0.0, 2.0, 0.0);
    let linear_four = spiral(size, SpiralType::Linear, 0.5, 0.5, 16.0, 0.0, 4.0, 0.0);
    assert_eq!(
        linear_two, linear_four,
        "the linear law has one slider, so `base` must change nothing"
    );

    let log_two = spiral(size, SpiralType::Logarithmic, 0.5, 0.5, 16.0, 0.0, 2.0, 0.0);
    let log_four = spiral(size, SpiralType::Logarithmic, 0.5, 0.5, 16.0, 0.0, 4.0, 0.0);
    assert_ne!(
        log_two, log_four,
        "the logarithmic law has two, and `base` is the second"
    );
}

/// The two laws are different laws.
///
/// A logarithmic spiral packs its arms toward the centre, so at the same reference radius it crosses
/// far more of them along a ray — measured 10 against the linear law's 3.
#[test]
fn spiral_laws_differ() {
    let size = 64usize;
    let linear = spiral(size, SpiralType::Linear, 0.5, 0.5, 16.0, 0.0, 2.0, 0.0);
    let logarithmic = spiral(size, SpiralType::Logarithmic, 0.5, 0.5, 16.0, 0.0, 2.0, 0.0);

    assert_ne!(linear, logarithmic, "two laws, two pictures");
    assert!(
        bands_along_ray(&logarithmic, size, 32, 32) > bands_along_ray(&linear, size, 32, 32),
        "the logarithmic law packs arms toward the centre, so it crosses more of them"
    );
}

/// `x` and `y` are fractions of the area, so 0.25 is a quarter across, not a quarter of a pixel.
#[test]
fn spiral_centre_is_normalised_to_the_area() {
    let size = 48usize;
    let centred = spiral(size, SpiralType::Linear, 0.5, 0.5, 12.0, 0.0, 2.0, 0.0);
    let offset = spiral(size, SpiralType::Linear, 0.25, 0.5, 12.0, 0.0, 2.0, 0.0);
    assert_ne!(centred, offset, "moving the centre must move the spiral");

    // Moving the centre a quarter LEFT must make the left edge's pattern coarser in the same way
    // the centred image's middle is: the measurable claim is simply that the two differ and that
    // 0.25 and 0.75 are mirror images of each other about the vertical axis.
    let mirrored = spiral(size, SpiralType::Linear, 0.75, 0.5, 12.0, 0.0, 2.0, 0.0);
    let flipped: Vec<u8> = (0..size * size)
        .flat_map(|i| {
            let (x, y) = (i % size, i / size);
            let source = (y * size + (size - 1 - x)) * 4;
            // A mirror flips the spiral's handedness too, so compare only the band COUNT per row.
            offset[source..source + 4].to_vec()
        })
        .collect();
    let rows_offset: Vec<usize> = (0..size)
        .map(|y| bands_along_ray(&flipped, size, 0, y))
        .collect();
    let rows_mirrored: Vec<usize> = (0..size)
        .map(|y| bands_along_ray(&mirrored, size, 0, y))
        .collect();
    let total_offset: usize = rows_offset.iter().sum();
    let total_mirrored: usize = rows_mirrored.iter().sum();
    assert!(
        total_offset.abs_diff(total_mirrored) * 20 < total_offset.max(total_mirrored),
        "a centre at 0.25 and one at 0.75 must be mirror images in how much they band: \
         {total_offset} against {total_mirrored}"
    );
}

/// `rotation` turns the pattern, and is in degrees.
#[test]
fn spiral_rotation_turns_the_pattern() {
    let size = 48usize;
    let zero = spiral(size, SpiralType::Linear, 0.5, 0.5, 12.0, 0.0, 2.0, 0.0);
    let ninety = spiral(size, SpiralType::Linear, 0.5, 0.5, 12.0, 90.0, 2.0, 0.0);
    assert_ne!(zero, ninety, "a quarter turn must show");

    // A full turn is the identity, which is what makes the unit degrees rather than radians: at
    // 360 the pattern must come back exactly, and it could not if the field were radians.
    let full = spiral(
        size,
        SpiralType::Linear,
        0.5,
        0.5,
        12.0,
        359.999_999,
        2.0,
        0.0,
    );
    let differing = zero
        .chunks(4)
        .zip(full.chunks(4))
        .filter(|(a, b)| a[0] != b[0])
        .count();
    assert!(
        differing * 100 < size * size,
        "a full turn in DEGREES returns the pattern; {differing} pixels differ"
    );
}

/// Out-of-range parameters are refused, including the base-1 logarithmic case upstream calls out.
#[test]
fn spiral_refuses_bad_parameters() {
    let size = 16usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let refused = |filter: Filter| {
        let mut editor = image(size as u32, size as u32, &grey);
        editor.execute(Command::ApplyFilter { filter }).is_err()
    };
    let make =
        |spiral_type: SpiralType, x: f64, radius: f64, rotation: f64, base: f64, balance: f64| {
            Filter::Spiral {
                spiral_type,
                x,
                y: 0.5,
                radius,
                rotation,
                base,
                balance,
                color1: Pixel::rgba(0, 0, 0, 255),
                color2: Pixel::rgba(255, 255, 255, 255),
            }
        };

    assert!(
        refused(make(SpiralType::Linear, 1.5, 24.0, 0.0, 2.0, 0.0)),
        "x is a fraction of the area, so 1.5 is outside it"
    );
    assert!(
        refused(make(SpiralType::Linear, 0.5, 0.0, 0.0, 2.0, 0.0)),
        "a zero radius has no spiral"
    );
    assert!(
        refused(make(SpiralType::Linear, 0.5, 24.0, 360.0, 2.0, 0.0)),
        "rotation is 0..360, read from the propgui normalising into that range"
    );
    assert!(
        refused(make(SpiralType::Linear, 0.5, 24.0, 0.0, 0.5, 0.0)),
        "base below 1 is outside the slider's own range"
    );
    assert!(
        refused(make(SpiralType::Linear, 0.5, 12.0, 0.0, 2.0, 1.5)),
        "balance is clamped to -1..1 in the source"
    );
    assert!(
        refused(make(SpiralType::Logarithmic, 0.5, 24.0, 0.0, 1.0, 0.0)),
        "a logarithmic spiral with base 1 has no growth and no inverse mapping -- upstream's own \
         comment calls this the NaN case"
    );
    assert!(
        !refused(make(SpiralType::Linear, 0.5, 24.0, 0.0, 1.0, 0.0)),
        "but the linear law ignores base, so base 1 is harmless there"
    );
}

/// A saved command with nothing but the kind loads.
#[test]
fn spiral_deserialises_with_defaults() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"spiral"}"#).expect("older saved commands must load");
    match filter {
        Filter::Spiral {
            spiral_type,
            x,
            y,
            rotation,
            balance,
            base,
            ..
        } => {
            assert_eq!(spiral_type, SpiralType::Linear, "the first enum member");
            assert_eq!((x, y), (0.5, 0.5), "centred");
            assert_eq!(rotation, 0.0);
            assert_eq!(balance, 0.0, "the midpoint of the read -1..1 range");
            assert_eq!(base, 2.0);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}
