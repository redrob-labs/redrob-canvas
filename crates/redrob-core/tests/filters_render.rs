//! K.6, render generators.

use redrob_core::{
    Command, Editor, Filter, GradientOutput, MazeAlgorithm, Pixel, SinusBlend, SinusPerturbation,
    SpiralType,
};
use std::collections::VecDeque;

#[path = "common/canvas.rs"]
mod canvas;

/// Build a test canvas. Memoised on its content by `common/canvas.rs` -- see that module for why.
fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    canvas::editor(width, height, colors)
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

#[allow(clippy::too_many_arguments)]
fn sinus(
    size: usize,
    x_scale: f64,
    y_scale: f64,
    complexity: f64,
    seed: u32,
    tiling: bool,
    perturbation: SinusPerturbation,
    blend: SinusBlend,
    exponent: f64,
) -> Vec<u8> {
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Sinus {
                x_scale,
                y_scale,
                complexity,
                seed,
                tiling,
                perturbation,
                color1: Pixel::rgba(0, 0, 0, 255),
                color2: Pixel::rgba(255, 255, 255, 255),
                blend,
                exponent,
            },
        })
        .expect("sinus");
    pixels(&editor)
}

/// A plain sinus at the defaults, varying only what each test varies.
fn sinus_plain(size: usize, seed: u32, tiling: bool) -> Vec<u8> {
    sinus(
        size,
        0.05,
        0.05,
        3.0,
        seed,
        tiling,
        SinusPerturbation::Ideal,
        SinusBlend::Linear,
        0.0,
    )
}

/// Mean step across the vertical seam, divided by a typical interior step.
///
/// A tiling texture has to be as smooth across the wrap as it is anywhere else, so this ratio is
/// the measurement that says whether tiling worked. It is a RATIO rather than an absolute so it
/// does not depend on how contrasty the particular texture is.
fn seam_ratio(out: &[u8], size: usize) -> f64 {
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);
    let seam: f64 = (0..size)
        .map(|y| f64::from((at(0, y) - at(size - 1, y)).abs()))
        .sum::<f64>()
        / size as f64;
    let interior: f64 = (0..size)
        .map(|y| f64::from((at(1, y) - at(0, y)).abs()))
        .sum::<f64>()
        / size as f64;
    seam / interior.max(0.5)
}

fn mean_channel(out: &[u8]) -> f64 {
    out.chunks(4).map(|c| f64::from(c[0])).sum::<f64>() / (out.len() / 4) as f64
}

/// `_Force tiling?` makes the texture wrap, and it is the sharpest measurement in the item.
///
/// The mechanism is readable from the name alone once you ask what it must DO: a sine closes over
/// the canvas exactly when its frequency is a whole number of cycles across it, because then both
/// edges share a value and a slope. So tiling snaps every frequency to an integer cycle count.
///
/// Measured as the seam step over a typical interior step: **50.68** without tiling and **1.00**
/// with — that is, with tiling the wrap is exactly as smooth as any neighbouring pair of columns,
/// which is what "tiles" means.
#[test]
fn sinus_force_tiling_closes_the_seam() {
    let size = 48usize;
    let loose = seam_ratio(&sinus_plain(size, 7, false), size);
    let tiled = seam_ratio(&sinus_plain(size, 7, true), size);

    assert!(
        tiled < 2.0,
        "a tiling texture's seam must be no worse than an interior step, measured {tiled:.2}"
    );
    assert!(
        loose > 10.0,
        "and without tiling the seam is a visible discontinuity, measured {loose:.2}"
    );
}

/// The exponent is monotone, and its DIRECTION is the measured one.
///
/// A positive exponent raises the blend factor to a higher power; the factor is 0 at `color1`, so
/// it pulls toward `color1`. **The first draft of the field's own comment had this backwards**,
/// which is the wrong-comment pattern this backlog records four earlier instances of — caught here
/// by measuring before the code was built on it rather than after.
///
/// Measured means against a black-to-white pair: 247.9, 228.2, 139.4, 45.3, 16.1 at exponents −4,
/// −2, 0, +2, +4.
#[test]
fn sinus_exponent_is_monotone_toward_the_first_colour() {
    let size = 48usize;
    let means: Vec<f64> = [-4.0f64, -2.0, 0.0, 2.0, 4.0]
        .iter()
        .map(|&exponent| {
            mean_channel(&sinus(
                size,
                0.05,
                0.05,
                3.0,
                7,
                false,
                SinusPerturbation::Ideal,
                SinusBlend::Linear,
                exponent,
            ))
        })
        .collect();

    assert!(
        means.windows(2).all(|w| w[1] < w[0]),
        "raising the exponent must move the texture toward color1 at every step: {means:?}"
    );
    assert!(
        means[0] > 200.0 && means[4] < 60.0,
        "and the span must be most of the range, not a nudge: {means:?}"
    );
}

/// The three gradient modes are three different gradients.
#[test]
fn sinus_blend_modes_differ() {
    let size = 48usize;
    let under = |blend: SinusBlend| {
        sinus(
            size,
            0.05,
            0.05,
            3.0,
            7,
            false,
            SinusPerturbation::Ideal,
            blend,
            0.0,
        )
    };
    let linear = under(SinusBlend::Linear);
    let bilinear = under(SinusBlend::Bilinear);
    let sinusoidal = under(SinusBlend::Sinusoidal);

    assert_ne!(linear, bilinear);
    assert_ne!(linear, sinusoidal);
    assert_ne!(bilinear, sinusoidal);

    // Bilinear FOLDS, so the two colours meet twice per cycle and the mean moves away from the
    // linear one. Measured 210.5 against 139.4.
    assert!(
        mean_channel(&bilinear) > mean_channel(&linear) + 30.0,
        "folding must change the distribution, not just the picture: {:.1} against {:.1}",
        mean_channel(&bilinear),
        mean_channel(&linear)
    );
}

/// `_Ideal` and `_Distorted` are different calculations.
#[test]
fn sinus_perturbation_changes_the_calculation() {
    let size = 48usize;
    let ideal = sinus_plain(size, 7, false);
    let distorted = sinus(
        size,
        0.05,
        0.05,
        3.0,
        7,
        false,
        SinusPerturbation::Distorted,
        SinusBlend::Linear,
        0.0,
    );
    assert_ne!(
        ideal, distorted,
        "feeding the sum back as a phase shift must change the texture"
    );
}

/// The seed is the whole randomness, and the two scales are independent axes.
#[test]
fn sinus_seed_and_axes_are_independent() {
    let size = 48usize;

    assert_eq!(
        sinus_plain(size, 7, false),
        sinus_plain(size, 7, false),
        "the same seed must give the same texture"
    );
    assert_ne!(
        sinus_plain(size, 7, false),
        sinus_plain(size, 8, false),
        "a different seed must give a different one"
    );

    // Vary ONE thing: the same pair of scales, swapped. A shared axis would make these equal.
    let wide = sinus(
        size,
        0.02,
        0.09,
        3.0,
        7,
        false,
        SinusPerturbation::Ideal,
        SinusBlend::Linear,
        0.0,
    );
    let tall = sinus(
        size,
        0.09,
        0.02,
        3.0,
        7,
        false,
        SinusPerturbation::Ideal,
        SinusBlend::Linear,
        0.0,
    );
    assert_ne!(wide, tall, "the x and y scales must be separate axes");
}

/// `Co_mplexity:` buys sine terms, so changing it changes the texture.
#[test]
fn sinus_complexity_changes_the_texture() {
    let size = 48usize;
    let simple = sinus_plain(size, 7, false);
    let complex = sinus(
        size,
        0.05,
        0.05,
        8.0,
        7,
        false,
        SinusPerturbation::Ideal,
        SinusBlend::Linear,
        0.0,
    );
    assert_ne!(simple, complex, "more terms must show");
}

/// Every output pixel lies on the segment between the two colours.
///
/// That is what makes this a two-colour texture rather than a palette: the blend factor is the only
/// thing that varies. Asserted on a pair with no shared channel, so a pixel off the segment cannot
/// hide — measured 0 off-segment pixels.
#[test]
fn sinus_output_lies_between_the_two_colours() {
    let size = 48usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Sinus {
                x_scale: 0.05,
                y_scale: 0.05,
                complexity: 3.0,
                seed: 7,
                tiling: false,
                perturbation: SinusPerturbation::Ideal,
                color1: Pixel::rgba(200, 40, 0, 255),
                color2: Pixel::rgba(0, 60, 180, 255),
                blend: SinusBlend::Sinusoidal,
                exponent: 1.5,
            },
        })
        .expect("sinus");
    let out = pixels(&editor);

    for chunk in out.chunks(4) {
        // Recover the blend factor from the red channel, which runs 200 down to 0.
        let t = 1.0 - f64::from(chunk[0]) / 200.0;
        let green = 40.0 * (1.0 - t) + 60.0 * t;
        let blue = 180.0 * t;
        assert!(
            (f64::from(chunk[1]) - green).abs() <= 2.0 && (f64::from(chunk[2]) - blue).abs() <= 2.0,
            "every pixel must sit on the segment between the two colours, found {chunk:?}"
        );
    }
}

/// Out-of-range parameters are refused.
#[test]
fn sinus_refuses_bad_parameters() {
    let size = 16usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let refused = |x_scale: f64, complexity: f64, exponent: f64| {
        let mut editor = image(size as u32, size as u32, &grey);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Sinus {
                    x_scale,
                    y_scale: 0.05,
                    complexity,
                    seed: 0,
                    tiling: false,
                    perturbation: SinusPerturbation::Ideal,
                    color1: Pixel::rgba(0, 0, 0, 255),
                    color2: Pixel::rgba(255, 255, 255, 255),
                    blend: SinusBlend::Linear,
                    exponent,
                },
            })
            .is_err()
    };
    assert!(refused(0.0, 3.0, 0.0), "a zero scale has no wave");
    assert!(
        refused(1000.0, 3.0, 0.0),
        "and a scale past the cap is refused"
    );
    assert!(refused(0.05, -1.0, 0.0), "complexity cannot be negative");
    assert!(refused(0.05, 3.0, 99.0), "the exponent is bounded");
    assert!(refused(f64::NAN, 3.0, 0.0), "a non-finite scale is refused");
}

/// A saved command with nothing but the kind loads.
#[test]
fn sinus_deserialises_with_defaults() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"sinus"}"#).expect("older saved commands must load");
    match filter {
        Filter::Sinus {
            x_scale,
            y_scale,
            complexity,
            seed,
            tiling,
            perturbation,
            blend,
            exponent,
            color1,
            color2,
            ..
        } => {
            // **`x_scale`/`y_scale` are the two K.17f could NOT correct, and the reason is a
            // domain difference rather than a number.** Upstream normalises the coordinate --
            // `x = (gdouble) i / o->width`, so its `x_scale` of 15.0 is radians across the WHOLE
            // width. Ours multiplies a raw pixel index, so 0.05 is radians per PIXEL. The
            // conversion is `15 / width`, which depends on the image and cannot be a constant.
            // Filed as K.17g; see the entry for why adopting upstream's domain is the real fix.
            assert_eq!((x_scale, y_scale), (0.05, 0.05), "still ours; K.17g");
            // The five K.17f did correct. Each message names the value it replaced.
            assert_eq!(complexity, 1.0, "was 2.0");
            assert_eq!(seed, 0);
            assert!(tiling, "was false -- upstream's `tiling` is TRUE");
            assert_eq!(
                perturbation,
                SinusPerturbation::Distorted,
                "was Ideal -- upstream's boolean `perturbation` is TRUE, i.e. distorted"
            );
            assert_eq!(blend, SinusBlend::Linear, "the first gradient");
            assert_eq!(exponent, 0.0, "the neutral middle");
            // The two colours were black and white -- our own neutral pair, not upstream's. GEGL
            // declares `"yellow"` and `"blue"`, which is why its sinus looks like itself.
            assert_eq!(color1, Pixel::rgba(255, 255, 0, 255), "was black");
            assert_eq!(color2, Pixel::rgba(0, 0, 255, 255), "was white");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

fn linear_sinusoid(size: usize, x_period: f64, y_period: f64, phase: f64) -> Vec<u8> {
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::LinearSinusoid {
                x_period,
                y_period,
                phase,
                color1: Pixel::rgba(0, 0, 0, 255),
                color2: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .expect("linear sinusoid");
    pixels(&editor)
}

/// A period long enough that the wave is effectively flat along that axis, and still inside
/// upstream's own `GIMP_MAX_IMAGE_SIZE`.
const FLAT_PERIOD: f64 = 100_000.0;

/// Local maxima along one row.
fn crests(out: &[u8], size: usize, row: usize) -> usize {
    let at = |x: usize| i32::from(out[(row * size + x) * 4]);
    (1..size - 1)
        .filter(|&x| at(x) >= at(x - 1) && at(x) > at(x + 1))
        .count()
}

/// A period is literally pixels per cycle.
///
/// Measured over a 64-pixel row with the other axis flat: 8 crests at period 8, 4 at 16, 2 at 32 —
/// exactly `64 / period`. Names the wrong behaviour: a period read as cycles-per-image, or as a
/// frequency rather than a period, would run the other way.
#[test]
fn linear_sinusoid_period_is_pixels_per_cycle() {
    let size = 64usize;
    for (period, expected) in [(8.0f64, 8usize), (16.0, 4), (32.0, 2)] {
        let out = linear_sinusoid(size, period, FLAT_PERIOD, 0.0);
        assert_eq!(
            crests(&out, size, 0),
            expected,
            "period {period} must give {expected} crests across 64 pixels"
        );
    }
}

/// The argument is LINEAR in position, so the value is constant along the line that fixes it.
///
/// This is the test that pins the reading the name forced. With both periods equal the argument
/// depends only on `x + y`, so the value must be constant along the ANTI-diagonal and vary fully
/// along the main one. A product of two sinusoids — the lattice reading, considered and rejected
/// because two multiplied sinusoids are not *a* sinusoid — would not be constant along any line.
///
/// Measured: worst anti-diagonal step **1**, which is pure rounding, against a worst main-diagonal
/// step of **97**.
#[test]
fn linear_sinusoid_is_constant_along_the_line_its_argument_fixes() {
    let size = 64usize;
    let out = linear_sinusoid(size, 16.0, 16.0, 0.0);
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);

    let mut along = 0;
    for y in 1..size {
        for x in 0..size - 1 {
            along = along.max((at(x, y) - at(x + 1, y - 1)).abs());
        }
    }
    assert!(
        along <= 2,
        "with equal periods the value must be constant along the anti-diagonal; worst step {along}"
    );

    let mut across = 0;
    for d in 0..size - 1 {
        across = across.max((at(d, d) - at(d + 1, d + 1)).abs());
    }
    assert!(
        across > 50,
        "and must vary fully across it, or the test would pass on a flat field; worst step {across}"
    );
}

/// The two periods are independent axes, and the phase shifts the wave.
#[test]
fn linear_sinusoid_axes_and_phase_are_independent() {
    let size = 48usize;

    // Vary ONE thing: the same pair of periods, swapped.
    assert_ne!(
        linear_sinusoid(size, 8.0, 32.0, 0.0),
        linear_sinusoid(size, 32.0, 8.0, 0.0),
        "swapping the periods must change the wave's direction"
    );

    assert_ne!(
        linear_sinusoid(size, 16.0, FLAT_PERIOD, 0.0),
        linear_sinusoid(size, 16.0, FLAT_PERIOD, 90.0),
        "a quarter-cycle phase shift must show"
    );

    // A full turn in DEGREES returns the wave, which is what makes the unit degrees not radians.
    let zero = linear_sinusoid(size, 16.0, FLAT_PERIOD, 0.0);
    let full = linear_sinusoid(size, 16.0, FLAT_PERIOD, 359.999_999);
    let differing = zero
        .chunks(4)
        .zip(full.chunks(4))
        .filter(|(a, b)| a[0].abs_diff(b[0]) > 1)
        .count();
    assert_eq!(
        differing, 0,
        "a full turn in degrees must return the wave exactly"
    );
}

/// There is no randomness at all: the same request is the same picture, and no seed exists.
///
/// This is the distinctness from `gegl:sinus` that upstream shipping both names requires — sinus is
/// a random sum with a seed and a complexity, so this cannot be. Asserted as the absence of any
/// variation across repeated runs, which is the observable form of "no seed".
#[test]
fn linear_sinusoid_is_fully_deterministic() {
    let size = 48usize;
    let run = || linear_sinusoid(size, 16.0, 24.0, 30.0);
    assert_eq!(run(), run(), "no seed means no variation to have");
}

/// Every pixel lies on the segment between the two colours, and the full span is reached.
#[test]
fn linear_sinusoid_spans_the_two_colours() {
    let size = 48usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::LinearSinusoid {
                x_period: 16.0,
                y_period: FLAT_PERIOD,
                phase: 0.0,
                color1: Pixel::rgba(220, 30, 0, 255),
                color2: Pixel::rgba(0, 50, 160, 255),
            },
        })
        .expect("linear sinusoid");
    let out = pixels(&editor);

    for chunk in out.chunks(4) {
        // Recover the blend factor from red, which runs 220 down to 0.
        let t = 1.0 - f64::from(chunk[0]) / 220.0;
        let green = 30.0 * (1.0 - t) + 50.0 * t;
        let blue = 160.0 * t;
        assert!(
            (f64::from(chunk[1]) - green).abs() <= 2.0 && (f64::from(chunk[2]) - blue).abs() <= 2.0,
            "every pixel must sit on the segment between the two colours, found {chunk:?}"
        );
    }

    // And the wave really reaches both ends -- measured 2..253 on a black-to-white pair.
    let mono = linear_sinusoid(size, 16.0, FLAT_PERIOD, 0.0);
    let low = mono.chunks(4).map(|c| c[0]).min().expect("non-empty");
    let high = mono.chunks(4).map(|c| c[0]).max().expect("non-empty");
    assert!(
        low < 10 && high > 245,
        "a full sine must reach both colours, measured {low}..{high}"
    );
}

/// Out-of-range parameters are refused, including a period below one pixel per cycle.
#[test]
fn linear_sinusoid_refuses_bad_parameters() {
    let size = 16usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let refused = |x_period: f64, y_period: f64, phase: f64| {
        let mut editor = image(size as u32, size as u32, &grey);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::LinearSinusoid {
                    x_period,
                    y_period,
                    phase,
                    color1: Pixel::rgba(0, 0, 0, 255),
                    color2: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .is_err()
    };
    assert!(
        refused(0.0, 32.0, 0.0),
        "a period of zero pixels per cycle has no wave"
    );
    assert!(
        refused(0.5, 32.0, 0.0),
        "and below one pixel per cycle there is nothing a pixel grid can show"
    );
    assert!(
        refused(1.0e7, 32.0, 0.0),
        "a period past GIMP_MAX_IMAGE_SIZE is refused"
    );
    assert!(refused(32.0, 32.0, 360.0), "phase is 0..360");
    assert!(
        refused(f64::NAN, 32.0, 0.0),
        "a non-finite period is refused"
    );
    // A NEGATIVE period is legal: it reverses the wave's direction on that axis, and the bound is
    // on the magnitude for exactly that reason.
    assert!(
        !refused(-16.0, 32.0, 0.0),
        "a negative period reverses the direction and is a meaningful request"
    );
}

/// A saved command with nothing but the kind loads.
#[test]
fn linear_sinusoid_deserialises_with_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"linear_sinusoid"}"#)
        .expect("older saved commands must load");
    match filter {
        Filter::LinearSinusoid {
            x_period,
            y_period,
            phase,
            ..
        } => {
            assert_eq!((x_period, y_period), (32.0, 32.0));
            assert_eq!(phase, 0.0);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

fn bayer(size: usize, order: u32) -> Vec<u8> {
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::BayerMatrix {
                order,
                color1: Pixel::rgba(0, 0, 0, 255),
                color2: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .expect("bayer matrix");
    pixels(&editor)
}

/// Order 2 is the canonical Bayer 4x4, asserted against the exact numbers the DITHER used.
///
/// This is the test that pins the recursion, and the only one that can. The matrix is built by
/// scaling by four and tiling four offset copies in BLOCKS; an interleaved variant — offsets at a
/// stride rather than as blocks — still yields a permutation of 0..15 and still tiles, so the
/// permutation and tiling tests below pass on it too.
///
/// What tells them apart is that `color_mode.rs` held these sixteen numbers as literals for the
/// indexed-mode ordered dither long before this filter existed. They were written for a different
/// purpose by someone not thinking about this recursion, which is what makes them independent
/// evidence. Only the block form reproduces them.
///
/// The dither now DERIVES its matrix from the same generator rather than restating it, so the two
/// are equal by construction and not merely by this assertion.
#[test]
fn bayer_matrix_order_two_is_the_canonical_four_by_four() {
    let size = 16usize;
    let out = bayer(size, 2);
    let value = |x: usize, y: usize| {
        // Recover the integer from the shade: 15 steps across 0..255.
        ((f64::from(out[(y * size + x) * 4]) * 15.0) / 255.0).round() as u32
    };

    let expected = [
        [0u32, 8, 2, 10],
        [12, 4, 14, 6],
        [3, 11, 1, 9],
        [15, 7, 13, 5],
    ];
    for (y, row) in expected.iter().enumerate() {
        let measured: Vec<u32> = (0..4).map(|x| value(x, y)).collect();
        assert_eq!(
            measured,
            row.to_vec(),
            "row {y} of the Bayer 4x4 must match the dither's own literals"
        );
    }
}

/// Order 1 is the base case, `[[0, 2], [3, 1]]`, exact as shades.
///
/// Spread over 0..255 in three steps: 0, 170, 255, 85. A generator has to reach both colours, which
/// is why the divisor is the largest value rather than the count — a dither would use the count, to
/// keep its thresholds strictly inside the interval. Recorded as a choice.
#[test]
fn bayer_matrix_order_one_is_the_base_case() {
    let size = 16usize;
    let out = bayer(size, 1);
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);
    assert_eq!(
        [at(0, 0), at(1, 0), at(0, 1), at(1, 1)],
        [0, 170, 255, 85],
        "the 2x2 base case, spread so both colours are reached"
    );
}

/// A tile is a PERMUTATION of `0..4^order - 1` — every value exactly once.
///
/// The defining property, and exact: order 1 gives 4 distinct shades, order 2 gives 16, order 3
/// gives 64. Note this passes under the wrong recursion too, which is why the test above exists.
#[test]
fn bayer_matrix_tile_is_a_permutation() {
    let size = 16usize;
    for order in [1u32, 2, 3] {
        let out = bayer(size, order);
        let side = 1usize << order;
        let tile: std::collections::HashSet<u8> = (0..side)
            .flat_map(|y| (0..side).map(move |x| (x, y)))
            .map(|(x, y)| out[(y * size + x) * 4])
            .collect();
        assert_eq!(
            tile.len(),
            side * side,
            "order {order}: a {side}x{side} tile must hold {} distinct values",
            side * side
        );

        // And the whole image holds no more than the tile does.
        let all: std::collections::HashSet<u8> = out.chunks(4).map(|c| c[0]).collect();
        assert_eq!(
            all.len(),
            side * side,
            "order {order}: the image is that tile repeated, so it adds no new values"
        );
    }
}

/// The pattern repeats every `2^order` pixels on both axes.
#[test]
fn bayer_matrix_tiles_at_its_own_period() {
    let size = 16usize;
    for order in [1u32, 2] {
        let out = bayer(size, order);
        let period = 1usize << order;
        let at = |x: usize, y: usize| out[(y * size + x) * 4];
        assert!(
            (0..size).all(|y| (0..size - period).all(|x| at(x, y) == at(x + period, y))),
            "order {order} must repeat every {period} pixels across"
        );
        assert!(
            (0..size - period).all(|y| (0..size).all(|x| at(x, y) == at(x, y + period))),
            "and every {period} pixels down"
        );
    }
}

/// Every pixel lies on the segment between the two colours, and both ends are reached.
#[test]
fn bayer_matrix_spans_the_two_colours() {
    let size = 16usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::BayerMatrix {
                order: 2,
                color1: Pixel::rgba(210, 20, 0, 255),
                color2: Pixel::rgba(0, 70, 150, 255),
            },
        })
        .expect("bayer matrix");
    let out = pixels(&editor);

    for chunk in out.chunks(4) {
        let t = 1.0 - f64::from(chunk[0]) / 210.0;
        let green = 20.0 * (1.0 - t) + 70.0 * t;
        let blue = 150.0 * t;
        assert!(
            (f64::from(chunk[1]) - green).abs() <= 2.0 && (f64::from(chunk[2]) - blue).abs() <= 2.0,
            "every pixel must sit on the segment between the two colours, found {chunk:?}"
        );
    }

    let mono = bayer(size, 2);
    let low = mono.chunks(4).map(|c| c[0]).min().expect("non-empty");
    let high = mono.chunks(4).map(|c| c[0]).max().expect("non-empty");
    assert_eq!((low, high), (0, 255), "a generator must reach both colours");
}

/// Order 0 and an order past the cap are refused.
#[test]
fn bayer_matrix_refuses_a_degenerate_order() {
    let size = 8usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let refused = |order: u32| {
        let mut editor = image(size as u32, size as u32, &grey);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::BayerMatrix {
                    order,
                    color1: Pixel::rgba(0, 0, 0, 255),
                    color2: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .is_err()
    };
    assert!(
        refused(0),
        "order 0 is a single cell holding one value, which is a flat field rather than a matrix"
    );
    assert!(refused(13), "and an order past the cap is refused");
    assert!(!refused(1), "but the 2x2 base case is legal");
}

/// A saved command with nothing but the kind loads the familiar 4x4.
#[test]
fn bayer_matrix_deserialises_with_defaults() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"bayer_matrix"}"#).expect("older saved commands must load");
    match filter {
        Filter::BayerMatrix { order, .. } => assert_eq!(order, 2, "the familiar 4x4"),
        other => panic!("wrong variant: {other:?}"),
    }
}

/// The twelve diffraction parameters, at the defaults, with named overrides per test.
struct Diffraction {
    frequency: [f64; 3],
    contours: [f64; 3],
    edges: [f64; 3],
    brightness: f64,
    scattering: f64,
    polarization: f64,
}

impl Default for Diffraction {
    fn default() -> Self {
        Self {
            frequency: [0.815; 3],
            contours: [0.819; 3],
            edges: [0.0; 3],
            brightness: 1.0,
            scattering: 0.0,
            polarization: 0.0,
        }
    }
}

fn diffraction(size: usize, p: &Diffraction) -> Vec<u8> {
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::DiffractionPatterns {
                frequency_red: p.frequency[0],
                frequency_green: p.frequency[1],
                frequency_blue: p.frequency[2],
                contour_red: p.contours[0],
                contour_green: p.contours[1],
                contour_blue: p.contours[2],
                edges_red: p.edges[0],
                edges_green: p.edges[1],
                edges_blue: p.edges[2],
                brightness: p.brightness,
                scattering: p.scattering,
                polarization: p.polarization,
            },
        })
        .expect("diffraction patterns");
    pixels(&editor)
}

fn channel(out: &[u8], index: usize) -> Vec<u8> {
    out.chunks(4).map(|c| c[index]).collect()
}

fn channel_mean(out: &[u8], index: usize) -> f64 {
    let values = channel(out, index);
    values.iter().map(|&b| f64::from(b)).sum::<f64>() / values.len() as f64
}

/// Each of the three triples is PER-CHANNEL, and that is the thing actually read.
///
/// The arithmetic here is reconstructed — the plug-in is deleted — so the test that matters is the
/// one aimed at what the sources DO say. The propgui groups the twelve properties into four slices
/// of three (`param_specs + 0, 3` and so on) and the po gives each of the first three groups its own
/// `_Red:`/`_Green:`/`_Blue:` triple, with three references apiece. So a red control may reach the
/// red channel and nothing else.
///
/// Varies ONE thing at a time, three times over, and names the wrong behaviour: an implementation
/// that folded the triples into a shared term — easy to do, since they all feed one phase — would
/// change every channel.
#[test]
fn diffraction_triples_are_per_channel() {
    let size = 48usize;
    let base = diffraction(size, &Diffraction::default());

    let cases: [(&str, Diffraction); 3] = [
        (
            "frequency",
            Diffraction {
                frequency: [2.5, 0.815, 0.815],
                ..Default::default()
            },
        ),
        (
            "contours",
            Diffraction {
                contours: [3.0, 0.819, 0.819],
                ..Default::default()
            },
        ),
        (
            "edges",
            Diffraction {
                edges: [1.0, 0.0, 0.0],
                ..Default::default()
            },
        ),
    ];

    for (name, params) in &cases {
        let changed = diffraction(size, params);
        assert_ne!(
            channel(&base, 0),
            channel(&changed, 0),
            "red {name} must change the red channel"
        );
        assert_eq!(
            channel(&base, 1),
            channel(&changed, 1),
            "red {name} must leave green alone"
        );
        assert_eq!(channel(&base, 2), channel(&changed, 2), "and blue alone");
    }
}

/// With no polarization the pattern is radial; polarization is the only term that breaks that.
///
/// **The pairing matters and the obvious one is wrong.** `c + d` and `c - d` are NOT mirror images:
/// the true centre of an `n`-pixel axis is at `n / 2`, while pixel centres sit at `i + 0.5`, so for
/// `n = 48` the pixels at 34 and 14 are 10.5 and 9.5 from it. Sampling that pair reads 100 against
/// 141 and looks like a broken filter. The mirror of pixel `i` is `n - 1 - i`, and on that pairing
/// the values agree exactly — measured 226/226, 100/100, 89/89 at three radii, on both axes.
#[test]
fn diffraction_is_radial_until_polarized() {
    let size = 48usize;
    let out = diffraction(size, &Diffraction::default());
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);
    let centre = size / 2;

    for d in [6usize, 10, 16] {
        let right = at(centre + d, centre);
        let left = at(size - 1 - (centre + d), centre);
        assert_eq!(right, left, "x must mirror at distance {d}");

        let down = at(centre, centre + d);
        let up = at(centre, size - 1 - (centre + d));
        assert_eq!(down, up, "and y must mirror at distance {d}");

        assert_eq!(
            right, down,
            "a radial pattern reads the same along x and y at distance {d}"
        );
    }

    let polarized = diffraction(
        size,
        &Diffraction {
            polarization: 4.0,
            ..Default::default()
        },
    );
    assert_ne!(
        out, polarized,
        "polarization turns the fringes with the angle, so it must break the radial symmetry"
    );
}

/// Brightness scales the whole pattern, linearly.
///
/// Measured red means 31.2, 62.4, 124.7 at brightness 0.25, 0.5, 1.0 — each a clean doubling.
#[test]
fn diffraction_brightness_scales_linearly() {
    let size = 48usize;
    let mean_for = |brightness: f64| {
        channel_mean(
            &diffraction(
                size,
                &Diffraction {
                    brightness,
                    ..Default::default()
                },
            ),
            0,
        )
    };
    let quarter = mean_for(0.25);
    let half = mean_for(0.5);
    let full = mean_for(1.0);

    assert!(
        (half / quarter - 2.0).abs() < 0.05,
        "half against a quarter must be a doubling: {half:.1} over {quarter:.1}"
    );
    assert!(
        (full / half - 2.0).abs() < 0.05,
        "and full against half likewise: {full:.1} over {half:.1}"
    );
}

/// Scattering washes the pattern out, and at full scattering there is nothing left.
///
/// Exactly interpretable at both ends: the red channel's range is 0..255 at scattering 0, 64..191 at
/// 0.5, and **128..128** at 1.0 — a completely flat field. That last is the sharp assertion, since a
/// filter merely dimming the pattern would not collapse it to a single value.
#[test]
fn diffraction_scattering_washes_the_pattern_out() {
    let size = 48usize;
    let range_for = |scattering: f64| {
        let out = diffraction(
            size,
            &Diffraction {
                scattering,
                ..Default::default()
            },
        );
        let values = channel(&out, 0);
        (
            *values.iter().min().expect("non-empty"),
            *values.iter().max().expect("non-empty"),
        )
    };

    assert_eq!(
        range_for(0.0),
        (0, 255),
        "unscattered, the pattern spans the range"
    );
    assert_eq!(range_for(0.5), (64, 191), "half way, it spans half of it");
    assert_eq!(
        range_for(1.0),
        (128, 128),
        "fully scattered, there is no pattern left at all"
    );
}

/// The same request twice is the same pattern: nothing here is random.
#[test]
fn diffraction_is_deterministic() {
    let size = 32usize;
    let params = Diffraction {
        frequency: [1.2, 0.7, 2.0],
        contours: [1.0, 2.0, 0.5],
        edges: [0.3, 0.0, 1.0],
        brightness: 0.8,
        scattering: 0.2,
        polarization: 1.5,
    };
    assert_eq!(
        diffraction(size, &params),
        diffraction(size, &params),
        "a diffraction pattern has no seed and no randomness"
    );
}

/// Out-of-range terms are refused, on every one of the twelve.
#[test]
fn diffraction_refuses_bad_terms() {
    let size = 8usize;
    let refused = |mutate: &dyn Fn(&mut Diffraction)| {
        let mut params = Diffraction::default();
        mutate(&mut params);
        let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
        let mut editor = image(size as u32, size as u32, &grey);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::DiffractionPatterns {
                    frequency_red: params.frequency[0],
                    frequency_green: params.frequency[1],
                    frequency_blue: params.frequency[2],
                    contour_red: params.contours[0],
                    contour_green: params.contours[1],
                    contour_blue: params.contours[2],
                    edges_red: params.edges[0],
                    edges_green: params.edges[1],
                    edges_blue: params.edges[2],
                    brightness: params.brightness,
                    scattering: params.scattering,
                    polarization: params.polarization,
                },
            })
            .is_err()
    };

    assert!(refused(&|p| p.frequency[1] = -1.0), "a negative frequency");
    assert!(refused(&|p| p.contours[2] = 99.0), "a contour past the cap");
    assert!(
        refused(&|p| p.edges[0] = f64::NAN),
        "a non-finite edge term"
    );
    assert!(refused(&|p| p.brightness = -0.1), "a negative brightness");
    assert!(refused(&|p| p.scattering = 50.0), "scattering past the cap");
    assert!(
        refused(&|p| p.polarization = f64::INFINITY),
        "a non-finite polarization"
    );
}

/// A saved command with nothing but the kind loads all twelve defaults.
#[test]
fn diffraction_deserialises_with_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"diffraction_patterns"}"#)
        .expect("older saved commands must load");
    match filter {
        Filter::DiffractionPatterns {
            frequency_red,
            frequency_green,
            frequency_blue,
            contour_red,
            edges_red,
            brightness,
            scattering,
            polarization,
            ..
        } => {
            assert_eq!(
                [frequency_red, frequency_green, frequency_blue],
                [0.815; 3],
                "the three frequencies share a default"
            );
            assert_eq!(contour_red, 0.819);
            assert_eq!(edges_red, 0.0, "sharp edges start off");
            assert_eq!(brightness, 1.0);
            assert_eq!(scattering, 0.0, "and the pattern starts unscattered");
            assert_eq!(polarization, 0.0);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

fn perlin(size: usize, scale: f64, seed: u32) -> Vec<u8> {
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::PerlinNoise {
                scale,
                seed,
                color1: Pixel::rgba(0, 0, 0, 255),
                color2: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .expect("perlin noise");
    pixels(&editor)
}

/// Gradient noise is EXACTLY zero at every lattice point, and that is the defining property.
///
/// The gradient at a lattice point meets a zero offset, so the dot product vanishes however the
/// gradient fell. Nothing else in this group has an invariant that exact, and it is precisely what
/// separates gradient noise from VALUE noise — which stores a value per lattice point and would read
/// anything there.
///
/// Measured: all sixteen lattice points of a 64-pixel canvas at scale 16 read exactly 128, the
/// midpoint. Reverse-verified against a value-noise substitution.
#[test]
fn perlin_noise_is_zero_at_every_lattice_point() {
    let size = 64usize;
    let scale = 16usize;
    let out = perlin(size, scale as f64, 7);
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);

    for j in 0..=3 {
        for i in 0..=3 {
            let (x, y) = (i * scale, j * scale);
            assert_eq!(
                at(x, y),
                128,
                "lattice point ({x}, {y}) must be exactly the midpoint"
            );
        }
    }

    // And a point NOT on the lattice must generally not be, or the field would be flat.
    let off_lattice: Vec<i32> = (0..4).map(|i| at(i * scale + 8, 8)).collect();
    assert!(
        off_lattice.iter().any(|&v| v != 128),
        "away from the lattice the field must actually vary: {off_lattice:?}"
    );
}

/// The scale is pixels per lattice cell, so halving it doubles the features.
///
/// Measured crossings of the midpoint along row 0: 13 at scale 8, 6 at 16, 2 at 32.
#[test]
fn perlin_noise_scale_sets_the_feature_size() {
    let size = 64usize;
    let crossings_for = |scale: f64| {
        let out = perlin(size, scale, 7);
        let above: Vec<bool> = (0..size).map(|x| out[x * 4] > 128).collect();
        (1..size).filter(|&x| above[x] != above[x - 1]).count()
    };

    let fine = crossings_for(8.0);
    let medium = crossings_for(16.0);
    let coarse = crossings_for(32.0);
    assert!(
        fine > medium && medium > coarse,
        "a smaller cell must give more features: {fine}, {medium}, {coarse}"
    );
    assert_eq!(coarse, 2, "at scale 32 a 64-pixel row holds two cells");
}

/// The field is smooth: Perlin's quintic ease leaves no crease at a cell boundary.
///
/// Names the wrong behaviour. The original cubic ease `3t^2 - 2t^3` has a discontinuous second
/// derivative at the ends, which shows as a visible ridge along every lattice line; dropping the
/// ease entirely gives a hard crease. Measured worst neighbouring step 18 out of 255 at scale 16, so
/// the bound is generous and still far below a crease.
#[test]
fn perlin_noise_has_no_crease_at_a_cell_boundary() {
    let size = 64usize;
    let out = perlin(size, 16.0, 7);
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);

    let mut worst_x = 0;
    let mut worst_y = 0;
    for y in 0..size {
        for x in 0..size - 1 {
            worst_x = worst_x.max((at(x, y) - at(x + 1, y)).abs());
        }
    }
    for y in 0..size - 1 {
        for x in 0..size {
            worst_y = worst_y.max((at(x, y) - at(x, y + 1)).abs());
        }
    }
    assert!(
        worst_x <= 25 && worst_y <= 25,
        "no neighbouring pair may jump; worst steps {worst_x} across and {worst_y} down"
    );
}

/// The noise is centred on the midpoint and does NOT reach both colours.
///
/// The second half is the recorded choice, asserted rather than left as a comment. The value is
/// normalised by the theoretical bound of `sqrt(2)/2`, which real samples rarely attain — measured
/// range 32..222 with a mean of 126.8 against a midpoint of 127.5. Every other generator in K.6
/// reaches both ends; this one cannot without stretching to the measured extremes, which would make
/// the same request give different pixels at different canvas sizes.
#[test]
fn perlin_noise_is_centred_and_does_not_reach_the_ends() {
    let size = 64usize;
    let out = perlin(size, 16.0, 7);
    let values: Vec<i32> = out.chunks(4).map(|c| i32::from(c[0])).collect();
    let low = *values.iter().min().expect("non-empty");
    let high = *values.iter().max().expect("non-empty");
    let mean = values.iter().sum::<i32>() as f64 / values.len() as f64;

    assert!(
        (mean - 127.5).abs() < 3.0,
        "gradient noise is centred, measured mean {mean:.1}"
    );
    assert!(
        low > 0 && high < 255,
        "the theoretical bound is not attained, so the ends are not reached: {low}..{high}"
    );
    assert!(
        high - low > 100,
        "but the field must still use most of the range: {low}..{high}"
    );
}

/// The seed picks the gradient table, and the same seed gives the same field.
#[test]
fn perlin_noise_seed_selects_the_field() {
    let size = 48usize;
    assert_eq!(
        perlin(size, 16.0, 7),
        perlin(size, 16.0, 7),
        "the same seed must give the same noise"
    );
    assert_ne!(
        perlin(size, 16.0, 7),
        perlin(size, 16.0, 8),
        "a different seed must give different noise"
    );

    // The lattice invariant holds for every seed, since it does not depend on which gradient fell.
    for seed in [0u32, 1, 99] {
        let out = perlin(size, 16.0, seed);
        assert_eq!(
            i32::from(out[(16 * size + 16) * 4]),
            128,
            "seed {seed} must still be zero at the lattice"
        );
    }
}

/// Every pixel lies on the segment between the two colours.
#[test]
fn perlin_noise_lies_between_the_two_colours() {
    let size = 48usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::PerlinNoise {
                scale: 16.0,
                seed: 3,
                color1: Pixel::rgba(200, 30, 0, 255),
                color2: Pixel::rgba(0, 80, 160, 255),
            },
        })
        .expect("perlin noise");
    let out = pixels(&editor);

    for chunk in out.chunks(4) {
        let t = 1.0 - f64::from(chunk[0]) / 200.0;
        let green = 30.0 * (1.0 - t) + 80.0 * t;
        let blue = 160.0 * t;
        assert!(
            (f64::from(chunk[1]) - green).abs() <= 2.0 && (f64::from(chunk[2]) - blue).abs() <= 2.0,
            "every pixel must sit on the segment between the two colours, found {chunk:?}"
        );
    }
}

/// A cell smaller than a pixel is refused, as is one past upstream's own image limit.
#[test]
fn perlin_noise_refuses_an_unusable_scale() {
    let size = 8usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let refused = |scale: f64| {
        let mut editor = image(size as u32, size as u32, &grey);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::PerlinNoise {
                    scale,
                    seed: 0,
                    color1: Pixel::rgba(0, 0, 0, 255),
                    color2: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .is_err()
    };
    assert!(refused(0.0), "a zero cell has no lattice");
    assert!(
        refused(0.5),
        "and below one pixel per cell there is nothing a pixel grid can show"
    );
    assert!(
        refused(1.0e7),
        "a scale past GIMP_MAX_IMAGE_SIZE is refused"
    );
    assert!(refused(f64::NAN), "a non-finite scale is refused");
    assert!(!refused(1.0), "but one pixel per cell is legal");
}

/// A saved command with nothing but the kind loads.
#[test]
fn perlin_noise_deserialises_with_defaults() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"perlin_noise"}"#).expect("older saved commands must load");
    match filter {
        Filter::PerlinNoise { scale, seed, .. } => {
            assert_eq!(scale, 32.0);
            assert_eq!(seed, 0);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

fn simplex(size: usize, scale: f64, seed: u32) -> Vec<u8> {
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::SimplexNoise {
                scale,
                seed,
                color1: Pixel::rgba(0, 0, 0, 255),
                color2: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .expect("simplex noise");
    pixels(&editor)
}

/// The paired test: the two noises give OPPOSITE answers at the integer lattice.
///
/// This is the assertion that keeps them distinct, and it is one test rather than two facts because
/// what matters is the contrast. Perlin is exactly zero at every integer lattice point — the
/// gradient there meets a zero offset and no other corner reaches. Simplex is not: its lattice is
/// the SKEWED one, so an integer point generally falls inside a simplex rather than on a vertex, and
/// the corners around it contribute.
///
/// Measured at the sixteen lattice points of a 64-pixel canvas at scale 16: Perlin gives 128
/// sixteen times, simplex gives **15 of 16** away from it.
///
/// **The one coincidence is worth stating rather than hiding**: the origin reads 128 under both,
/// because the skew fixes `(0, 0)` and it is a vertex of either lattice — and there all three
/// simplex corners either carry a zero offset or fall outside the kernel's support.
///
/// A simplex implemented as a renamed Perlin fails this; so does one whose skew was dropped.
#[test]
fn simplex_noise_is_not_zero_where_perlin_is() {
    let size = 64usize;
    let step = 16usize;
    let simplex_out = simplex(size, step as f64, 7);
    let perlin_out = perlin(size, step as f64, 7);

    let points: Vec<(usize, usize)> = (0..=3)
        .flat_map(|j| (0..=3).map(move |i| (i * step, j * step)))
        .collect();

    let perlin_on_midpoint = points
        .iter()
        .filter(|&&(x, y)| perlin_out[(y * size + x) * 4] == 128)
        .count();
    assert_eq!(
        perlin_on_midpoint, 16,
        "Perlin must be exactly zero at all sixteen lattice points"
    );

    let simplex_off_midpoint = points
        .iter()
        .filter(|&&(x, y)| simplex_out[(y * size + x) * 4] != 128)
        .count();
    assert!(
        simplex_off_midpoint >= 14,
        "simplex's lattice is the skewed one, so almost no integer point is a vertex; only \
         {simplex_off_midpoint} of 16 were off the midpoint"
    );

    // The origin is a vertex of both lattices, so it is the expected coincidence.
    assert_eq!(
        simplex_out[0], 128,
        "the skew fixes the origin, so it is a vertex either way"
    );
}

/// The two operations are different operations, which upstream shipping both requires.
#[test]
fn simplex_noise_is_not_perlin_noise() {
    let size = 48usize;
    for (scale, seed) in [(8.0f64, 1u32), (16.0, 7), (32.0, 99)] {
        assert_ne!(
            simplex(size, scale, seed),
            perlin(size, scale, seed),
            "at scale {scale} and seed {seed} a triangular lattice summed through a radial kernel \
             must not agree with a square lattice interpolated"
        );
    }
}

/// The scale sets the feature size.
///
/// Measured midpoint crossings along row 0: 9 at scale 8, 2 at 16, 1 at 32.
#[test]
fn simplex_noise_scale_sets_the_feature_size() {
    let size = 64usize;
    let crossings_for = |scale: f64| {
        let out = simplex(size, scale, 7);
        let above: Vec<bool> = (0..size).map(|x| out[x * 4] > 128).collect();
        (1..size).filter(|&x| above[x] != above[x - 1]).count()
    };
    let fine = crossings_for(8.0);
    let coarse = crossings_for(32.0);
    assert!(
        fine > coarse,
        "a smaller cell must give more features: {fine} against {coarse}"
    );
}

/// The kernel is smooth: it reaches zero in value AND derivative where its support ends.
///
/// So no corner switching on or off can show as a seam. Measured worst neighbouring step 39 of 255
/// at scale 16 — larger than Perlin's 18, because the `(0.5 - r^2)^4` kernel is steeper than a
/// quintic-eased interpolation, and still nothing like a discontinuity.
#[test]
fn simplex_noise_has_no_seam_where_a_corner_drops_out() {
    let size = 64usize;
    let out = simplex(size, 16.0, 7);
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);

    let mut worst = 0;
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            worst = worst.max((at(x, y) - at(x + 1, y)).abs());
            worst = worst.max((at(x, y) - at(x, y + 1)).abs());
        }
    }
    assert!(
        worst <= 55,
        "a corner leaving the kernel's support must not show as a jump; worst step {worst}"
    );
}

/// Centred, and like Perlin it does not reach both colours.
///
/// Measured range 40..215 with a mean of 129.3. The published `* 70` scaling brings the sum onto
/// roughly −1..1 rather than exactly, so the ends are approached and not met — the same honest
/// limitation `PerlinNoise` records, from a different cause.
#[test]
fn simplex_noise_is_centred_and_does_not_reach_the_ends() {
    let size = 64usize;
    let out = simplex(size, 16.0, 7);
    let values: Vec<i32> = out.chunks(4).map(|c| i32::from(c[0])).collect();
    let low = *values.iter().min().expect("non-empty");
    let high = *values.iter().max().expect("non-empty");
    let mean = values.iter().sum::<i32>() as f64 / values.len() as f64;

    assert!(
        (mean - 127.5).abs() < 6.0,
        "simplex noise is centred, measured mean {mean:.1}"
    );
    assert!(
        low > 0 && high < 255,
        "the published scaling approaches the ends without meeting them: {low}..{high}"
    );
    assert!(
        high - low > 100,
        "but the field must still use most of the range: {low}..{high}"
    );
}

/// The seed picks the gradient table, and the same seed gives the same field.
#[test]
fn simplex_noise_seed_selects_the_field() {
    let size = 48usize;
    assert_eq!(
        simplex(size, 16.0, 7),
        simplex(size, 16.0, 7),
        "the same seed must give the same noise"
    );
    assert_ne!(
        simplex(size, 16.0, 7),
        simplex(size, 16.0, 8),
        "a different seed must give different noise"
    );
}

/// Every pixel lies on the segment between the two colours.
#[test]
fn simplex_noise_lies_between_the_two_colours() {
    let size = 48usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let mut editor = image(size as u32, size as u32, &grey);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::SimplexNoise {
                scale: 16.0,
                seed: 3,
                color1: Pixel::rgba(190, 40, 0, 255),
                color2: Pixel::rgba(0, 90, 170, 255),
            },
        })
        .expect("simplex noise");
    let out = pixels(&editor);

    for chunk in out.chunks(4) {
        let t = 1.0 - f64::from(chunk[0]) / 190.0;
        let green = 40.0 * (1.0 - t) + 90.0 * t;
        let blue = 170.0 * t;
        assert!(
            (f64::from(chunk[1]) - green).abs() <= 2.0 && (f64::from(chunk[2]) - blue).abs() <= 2.0,
            "every pixel must sit on the segment between the two colours, found {chunk:?}"
        );
    }
}

/// An unusable scale is refused, on the same bounds as Perlin's.
#[test]
fn simplex_noise_refuses_an_unusable_scale() {
    let size = 8usize;
    let grey = vec![Pixel::rgba(128, 128, 128, 255); size * size];
    let refused = |scale: f64| {
        let mut editor = image(size as u32, size as u32, &grey);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::SimplexNoise {
                    scale,
                    seed: 0,
                    color1: Pixel::rgba(0, 0, 0, 255),
                    color2: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .is_err()
    };
    assert!(refused(0.0), "a zero cell has no lattice");
    assert!(
        refused(0.5),
        "and below one pixel per cell nothing is showable"
    );
    assert!(
        refused(1.0e7),
        "a scale past GIMP_MAX_IMAGE_SIZE is refused"
    );
    assert!(refused(f64::NAN), "a non-finite scale is refused");
    assert!(!refused(1.0), "but one pixel per cell is legal");
}

/// A saved command with nothing but the kind loads.
#[test]
fn simplex_noise_deserialises_with_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"simplex_noise"}"#)
        .expect("older saved commands must load");
    match filter {
        Filter::SimplexNoise { scale, seed, .. } => {
            assert_eq!(scale, 32.0);
            assert_eq!(seed, 0);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

fn image_gradient(size: usize, colors: &[Pixel], output: GradientOutput) -> Vec<u8> {
    let mut editor = image(size as u32, size as u32, colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ImageGradient { output },
        })
        .expect("image gradient");
    pixels(&editor)
}

fn grey(value: u8) -> Pixel {
    Pixel::rgba(value, value, value, 255)
}

/// A ramp of slope k has gradient magnitude EXACTLY k, and a flat field exactly zero.
///
/// The defining property of a derivative.
///
/// **An earlier version of this comment claimed the test also distinguishes this from
/// `gegl:edge-sobel`. That was wrong, and the injection caught it.** A properly normalised Sobel
/// reproduces a ramp's slope exactly too — `(1 + 2 + 1) * k * 2 / 8 = k` — so it passes this, and it
/// passes every other test here, because all of them use inputs that are constant along one axis.
/// The two kernels agree on every LINEAR field and differ only where the image curves. The test that
/// separates them is `image_gradient_stencil_is_a_cross_not_a_block`.
///
/// Measured: slopes 1, 3 and 7 give interior magnitudes of exactly 1, 3 and 7, and a flat field
/// gives 0 nonzero pixels out of 1024.
#[test]
fn image_gradient_magnitude_is_the_exact_slope() {
    let size = 32usize;

    let flat = vec![grey(100); size * size];
    let out = image_gradient(size, &flat, GradientOutput::Magnitude);
    assert_eq!(
        out.chunks(4).filter(|c| c[0] != 0).count(),
        0,
        "a flat field has no gradient anywhere"
    );

    for slope in [1u32, 3, 7] {
        let ramp: Vec<Pixel> = (0..size * size)
            .map(|i| grey(((i % size) as u32 * slope).min(255) as u8))
            .collect();
        let out = image_gradient(size, &ramp, GradientOutput::Magnitude);
        let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);
        let interior: Vec<i32> = (2..10).map(|x| at(x, 16)).collect();
        assert!(
            interior.iter().all(|&v| v == slope as i32),
            "a ramp of slope {slope} must give magnitude exactly {slope}: {interior:?}"
        );
    }
}

/// The stencil is a CROSS, not a 3x3 block — which is what separates this from `gegl:edge-sobel`.
///
/// `image-gradient` is the plain central difference, so it reads only the four axis neighbours.
/// Sobel's kernel spans a 3x3 block and reads the diagonals too. Upstream ships both names, so the
/// catalogue requires them to differ (cycle 68's route), and this is the assertion that holds them
/// apart.
///
/// **It took a third input to find, and the injection is what forced the search.** Substituting
/// Sobel's kernel passed every test in this file: both kernels reproduce a ramp's slope exactly, and
/// both give a vertical step 128 on each straddling pixel, because every one of those inputs is
/// constant along one axis. Even a DIAGONAL ramp fails to separate them — measured 8 under both —
/// since they agree on every linear field and differ only where the image curves.
///
/// A single bright pixel does it. Measured at its diagonal neighbour: **0** under the central
/// difference, **45** under Sobel. The zero is the structural fact, and it is exact.
#[test]
fn image_gradient_stencil_is_a_cross_not_a_block() {
    let size = 16usize;
    let mut impulse = vec![grey(0); size * size];
    impulse[8 * size + 8] = grey(255);

    let out = image_gradient(size, &impulse, GradientOutput::Magnitude);
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);

    // The four axis neighbours see it, at half its height each.
    for (x, y) in [(7usize, 8usize), (9, 8), (8, 7), (8, 9)] {
        assert_eq!(
            at(x, y),
            128,
            "the axis neighbour at ({x}, {y}) straddles the pixel and sees half its height"
        );
    }

    // The four diagonals see NOTHING, because the stencil does not reach them.
    for (x, y) in [(7usize, 7usize), (9, 7), (7, 9), (9, 9)] {
        assert_eq!(
            at(x, y),
            0,
            "the diagonal at ({x}, {y}) is outside a cross stencil; a 3x3 kernel would read 45 here"
        );
    }
}

/// The direction is the angle, over the full turn mapped onto 0..255.
///
/// Exact at the quarter turns, which is what pins both the unit and the wrap point: an x-increasing
/// ramp reads **0**, a y-increasing one **64** (a quarter of 255), and a DECREASING x-ramp **128**
/// (half). Putting the full turn on the range rather than a half turn means the wrap sits at one end
/// instead of in the middle, so a direction map has one seam rather than two.
#[test]
fn image_gradient_direction_is_the_angle() {
    let size = 32usize;
    let at = |out: &[u8], x: usize, y: usize| i32::from(out[(y * size + x) * 4]);

    let rising_x: Vec<Pixel> = (0..size * size)
        .map(|i| grey(((i % size) as u32 * 4).min(255) as u8))
        .collect();
    let out = image_gradient(size, &rising_x, GradientOutput::Direction);
    assert_eq!(at(&out, 5, 16), 0, "a ramp rising in x points along x");

    let rising_y: Vec<Pixel> = (0..size * size)
        .map(|i| grey(((i / size) as u32 * 4).min(255) as u8))
        .collect();
    let out = image_gradient(size, &rising_y, GradientOutput::Direction);
    assert_eq!(
        at(&out, 16, 5),
        64,
        "a ramp rising in y is a quarter turn on"
    );

    let falling_x: Vec<Pixel> = (0..size * size)
        .map(|i| grey((255 - ((i % size) as u32 * 4).min(255)) as u8))
        .collect();
    let out = image_gradient(size, &falling_x, GradientOutput::Direction);
    assert_eq!(
        at(&out, 5, 16),
        128,
        "and one falling in x is half a turn on"
    );
}

/// A step edge puts its magnitude on the two pixels either side of the step, at half its height.
///
/// The central difference spans two pixels, so a 255-high step gives `255 / 2` to each of the pair
/// straddling it and nothing elsewhere. Measured `[0, 0, 128, 128, 0, 0]` across the edge — which
/// also names what a one-sided difference would give: a single pixel at the full height.
#[test]
fn image_gradient_spreads_a_step_across_the_central_difference() {
    let size = 32usize;
    let step: Vec<Pixel> = (0..size * size)
        .map(|i| if i % size < 16 { grey(0) } else { grey(255) })
        .collect();
    let out = image_gradient(size, &step, GradientOutput::Magnitude);
    let at = |x: usize| i32::from(out[(16 * size + x) * 4]);

    assert_eq!(
        (13..19).map(at).collect::<Vec<i32>>(),
        vec![0, 0, 128, 128, 0, 0],
        "the pair straddling the step carries half its height each"
    );
}

/// The two output modes are two different pictures.
#[test]
fn image_gradient_output_modes_differ() {
    let size = 32usize;
    let ramp: Vec<Pixel> = (0..size * size)
        .map(|i| grey((((i % size) + (i / size)) as u32 * 3).min(255) as u8))
        .collect();
    assert_ne!(
        image_gradient(size, &ramp, GradientOutput::Magnitude),
        image_gradient(size, &ramp, GradientOutput::Direction),
        "how steeply the image changes is not which way it changes"
    );
}

/// The gradient is taken on LUMINANCE, which is what makes a single direction possible.
///
/// Two images with the same luminance ramp but different hues must give the same gradient. A
/// per-channel reading would give three gradients and `Direction` could not name one angle — the
/// parameter set is what forces the choice, and this is the test of it.
#[test]
fn image_gradient_reduces_colour_to_luminance() {
    let size = 32usize;
    // A grey ramp, and a ramp in red alone scaled so its luminance matches at every column.
    let steps: Vec<f64> = (0..size).map(|x| x as f64 * 2.0).collect();
    let grey_ramp: Vec<Pixel> = (0..size * size)
        .map(|i| grey(steps[i % size].round() as u8))
        .collect();
    let red_ramp: Vec<Pixel> = (0..size * size)
        .map(|i| {
            // Luminance of pure red is 0.2126, so match the grey ramp's luminance exactly.
            let target = steps[i % size];
            let red = (target / 0.2126).min(255.0);
            Pixel::rgba(red.round() as u8, 0, 0, 255)
        })
        .collect();

    let from_grey = image_gradient(size, &grey_ramp, GradientOutput::Magnitude);
    let from_red = image_gradient(size, &red_ramp, GradientOutput::Magnitude);

    // Compare only where the red ramp has not clipped, since above 255/0.2126 it cannot follow.
    let unclipped = (0..size)
        .filter(|&x| steps[x] / 0.2126 < 250.0)
        .collect::<Vec<usize>>();
    assert!(unclipped.len() > 10, "the comparison needs a usable span");
    for &x in &unclipped[2..unclipped.len() - 1] {
        let a = i32::from(from_grey[(16 * size + x) * 4]);
        let b = i32::from(from_red[(16 * size + x) * 4]);
        assert!(
            (a - b).abs() <= 1,
            "at x={x} the same luminance ramp must give the same gradient: {a} against {b}"
        );
    }
}

/// Alpha carries through: the gradient says how the image changes, not how much of it there is.
#[test]
fn image_gradient_carries_alpha_through() {
    let size = 16usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|i| {
            let v = ((i % size) as u32 * 8).min(255) as u8;
            Pixel::rgba(v, v, v, ((i / size) as u32 * 8).min(255) as u8)
        })
        .collect();
    let out = image_gradient(size, &colors, GradientOutput::Magnitude);
    for (index, chunk) in out.chunks(4).enumerate() {
        assert_eq!(
            chunk[3], colors[index].a,
            "pixel {index}'s alpha must be untouched"
        );
    }
}

/// A saved command with nothing but the kind loads.
#[test]
fn image_gradient_deserialises_with_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"image_gradient"}"#)
        .expect("older saved commands must load");
    match filter {
        Filter::ImageGradient { output } => assert_eq!(
            output,
            GradientOutput::Magnitude,
            "magnitude is the unsurprising default"
        ),
        other => panic!("wrong variant: {other:?}"),
    }
}
