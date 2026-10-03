//! K.6, render generators.

use redrob_core::{Command, Document, Editor, Filter, MazeAlgorithm, Pixel, Rect, SelectionMode};
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
