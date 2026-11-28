//! Genre templates: small finished games the New project dialog starts from.
//! Each is made of components and blocks only, so it plays the same on the VM,
//! the compiled logic and a built player, and the corpus can hold it still.

use super::presets::PlayerPreset;
use super::spec::{BodyType, ColliderSpec, RigidbodySpec};
use crate::blocks::{Instruction, InstructionKind as K, Strand, VariableDef};
use crate::project::{Actor, Project, Scene};
use crate::scene::{Mode, Visual};
use crate::ui::UiAnchor;
use crate::value::{Evaluated, Op, Value};

fn strand(x: i32, kinds: Vec<K>) -> Strand {
    Strand::with_instructions(x, 0, kinds.into_iter().map(Instruction::new).collect())
}

fn body(kinds: Vec<K>) -> Vec<Instruction> {
    kinds.into_iter().map(Instruction::new).collect()
}

fn num(n: f64) -> Value {
    Value::number(n)
}

fn var(name: &str) -> Value {
    Value::Var {
        name: name.to_string(),
    }
}

fn op(op: Op, args: Vec<Value>) -> Value {
    Value::op(op, args)
}

fn global(project: &mut Project, name: &str, value: f64) {
    project.globals.push(VariableDef {
        name: name.to_string(),
        value: Evaluated::Number(value),
    });
}

/// Adds an actor with a collider shaped like its look. `body: None` is static
/// scenery.
fn place(
    scene: &mut Scene,
    library: &super::MaterialLibrary,
    actor: Actor,
    body: Option<BodyType>,
    trigger: bool,
) -> Result<String, String> {
    let id = scene.add_actor(actor);
    if let Some(body_type) = body {
        scene.set_rigidbody(
            &id,
            RigidbodySpec {
                body_type,
                ..RigidbodySpec::default()
            },
            library,
        )?;
    }
    let mut collider = ColliderSpec::from_look();
    collider.trigger = trigger;
    scene.add_collider(&id, collider, library)?;
    Ok(id)
}

fn actor_at(name: &str, visual: Visual, at: [f32; 3]) -> Actor {
    let mut actor = Actor::new(name, visual);
    actor.components.placement_mut().position = at;
    actor
}

fn rect(color: &str, w: f32, h: f32) -> Visual {
    Visual::Rect {
        color: color.into(),
        size: [w, h],
    }
}

fn cuboid(color: &str, size: [f32; 3]) -> Visual {
    Visual::Cuboid {
        color: color.into(),
        size,
    }
}

/// A pickup: touching the player counts it into `counter` and removes it.
/// `then` runs after the count, before it goes.
fn pickup_strand(counter: &str, then: Vec<K>) -> Strand {
    let mut kinds = vec![
        K::WhenCollision {
            with: "Player".to_string(),
            phase: super::ContactPhase::Enter,
            scope: Default::default(),
        },
        K::ChangeVariable {
            name: counter.to_string(),
            value: num(1.0),
        },
    ];
    kinds.extend(then);
    kinds.push(K::DeleteActor {
        target: Value::text(""),
    });
    strand(0, kinds)
}

/// The starter project with its player turned into `preset`, and the
/// physics document migrated to components.
fn start(
    name: &str,
    mode: Mode,
    preset: PlayerPreset,
    edit: impl FnOnce(&mut Scene),
) -> Result<(Project, super::MaterialLibrary, String), String> {
    let mut project = Project::starter(name, mode);
    let mut library = project.physics.materials.clone();
    let (scene, settings) = project.active_scene_and_settings();
    edit(scene);
    scene.migrate_physics(&mut library);
    let player = scene
        .actors
        .iter()
        .find(|a| a.name == "Player")
        .map(|a| a.id.clone())
        .ok_or("the starter project lost its player")?;
    scene.apply_player_preset(&mut settings.input, &player, preset, true, &library)?;
    Ok((project, library, player))
}

fn finish(mut project: Project, library: super::MaterialLibrary) -> Project {
    project.physics.materials = library;
    project.physics.stamp();
    project
}

/// A side-scrolling platformer: steps of platforms, coins to collect and a
/// flag at the end that says so.
pub(super) fn platformer(name: &str, mode: Mode) -> Result<Project, String> {
    if mode.is_3d() {
        return Err("the platformer template is 2D only".to_string());
    }
    let (mut project, library, _) = start(name, mode, PlayerPreset::Platformer2d, |scene| {
        if let Some(ground) = scene.actors.iter_mut().find(|a| a.name == "Ground") {
            ground.components.set_visual(rect("#3E4A5B", 2400.0, 40.0));
            ground.components.placement_mut().position = [800.0, -220.0, 0.0];
        }
    })?;
    global(&mut project, "coins", 0.0);
    global(&mut project, "finished", 0.0);
    let scene = project.active_scene_mut();
    // Each platform a jump above the last (the motor jumps about 58 px).
    let platforms = [
        (320.0, -165.0),
        (520.0, -125.0),
        (720.0, -85.0),
        (980.0, -125.0),
    ];
    for (x, top) in platforms {
        place(
            scene,
            &library,
            actor_at(
                "Platform",
                rect("#6D6875", 160.0, 20.0),
                [x, top - 10.0, 0.0],
            ),
            None,
            false,
        )?;
    }
    // One coin on the way in, then one over each platform.
    let mut coins = vec![(150.0, -175.0)];
    coins.extend(platforms.iter().map(|&(x, top)| (x, top + 30.0)));
    for (x, y) in coins {
        let mut coin = actor_at(
            "Coin",
            Visual::Circle {
                color: "#FFD166".into(),
                radius: 12.0,
            },
            [x, y, 0.0],
        );
        coin.graph.strands = vec![pickup_strand("coins", Vec::new())];
        place(scene, &library, coin, None, true)?;
    }
    let mut flag = actor_at("Flag", rect("#06D6A0", 20.0, 80.0), [1300.0, -160.0, 0.0]);
    flag.graph.strands = vec![strand(
        0,
        vec![
            K::WhenCollision {
                with: "Player".to_string(),
                phase: super::ContactPhase::Enter,
                scope: Default::default(),
            },
            K::SetVariable {
                name: "finished".to_string(),
                value: num(1.0),
            },
            K::Say {
                text: op(
                    Op::Join,
                    vec![
                        Value::text("Made it with "),
                        op(Op::Join, vec![var("coins"), Value::text(" coins!")]),
                    ],
                ),
            },
        ],
    )];
    place(scene, &library, flag, None, true)?;
    Ok(finish(project, library))
}

/// A top-down arena: walls, gems to collect, and a guard on patrol that sends
/// the player back to the start and costs a life.
pub(super) fn top_down(name: &str, mode: Mode) -> Result<Project, String> {
    if mode.is_3d() {
        return Err("the top-down template is 2D only".to_string());
    }
    let (mut project, library, player) = start(name, mode, PlayerPreset::TopDown2d, |scene| {
        // Seen from above, there is no floor to stand on.
        scene.actors.retain(|a| a.name != "Ground");
        scene.world.gravity = [0.0; 3];
        if let Some(player) = scene.actors.iter_mut().find(|a| a.name == "Player") {
            player.components.placement_mut().position = [0.0, 0.0, 0.0];
        }
    })?;
    global(&mut project, "gems", 0.0);
    global(&mut project, "lives", 3.0);
    let scene = project.active_scene_mut();
    scene.add_actor(actor_at(
        "Floor",
        rect("#2B2D42", 760.0, 560.0),
        [0.0, 0.0, -1.0],
    ));
    for (name, at, size) in [
        ("Wall", [0.0, 290.0], [780.0, 20.0]),
        ("Wall", [0.0, -290.0], [780.0, 20.0]),
        ("Wall", [-390.0, 0.0], [20.0, 600.0]),
        ("Wall", [390.0, 0.0], [20.0, 600.0]),
        ("Wall", [-160.0, 120.0], [20.0, 200.0]),
        ("Wall", [180.0, -100.0], [220.0, 20.0]),
    ] {
        place(
            scene,
            &library,
            actor_at(name, rect("#8D99AE", size[0], size[1]), [at[0], at[1], 0.0]),
            None,
            false,
        )?;
    }
    for (x, y) in [
        (150.0, 0.0),
        (-300.0, 220.0),
        (300.0, -220.0),
        (-280.0, -200.0),
    ] {
        let mut gem = actor_at(
            "Gem",
            Visual::Circle {
                color: "#4CC9F0".into(),
                radius: 10.0,
            },
            [x, y, 0.0],
        );
        gem.graph.strands = vec![pickup_strand(
            "gems",
            vec![K::If {
                condition: op(Op::Eq, vec![var("gems"), num(4.0)]),
                body: body(vec![K::Broadcast {
                    name: "all gems".to_string(),
                }]),
            }],
        )];
        place(scene, &library, gem, None, true)?;
    }
    let mut guard = actor_at("Guard", rect("#EF476F", 40.0, 40.0), [0.0, 200.0, 0.0]);
    let glide = |x: f64| K::Glide {
        seconds: num(2.0),
        x: num(x),
        y: num(200.0),
        z: num(0.0),
        easing: Default::default(),
    };
    guard.graph.strands = vec![
        strand(
            0,
            vec![
                K::WhenStarted,
                K::Forever {
                    body: body(vec![glide(300.0), glide(-100.0)]),
                },
            ],
        ),
        strand(
            320,
            vec![
                K::WhenMessage {
                    name: "all gems".to_string(),
                },
                K::Say {
                    text: Value::text("You got them all!"),
                },
            ],
        ),
    ];
    place(scene, &library, guard, Some(BodyType::Kinematic), true)?;
    // Caught: back to the start, one life fewer.
    let actor = scene.actor_mut(&player).ok_or("the player went missing")?;
    actor.graph.strands.push(strand(
        0,
        vec![
            K::WhenCollision {
                with: "Guard".to_string(),
                phase: super::ContactPhase::Enter,
                scope: Default::default(),
            },
            K::ChangeVariable {
                name: "lives".to_string(),
                value: num(-1.0),
            },
            K::GoTo {
                x: num(0.0),
                y: num(0.0),
                z: num(0.0),
            },
        ],
    ));
    Ok(finish(project, library))
}

/// A first-person room: orbs to collect, a stair up to the last one, and a
/// door that slides open once all four are in.
pub(super) fn first_person(name: &str, mode: Mode) -> Result<Project, String> {
    if !mode.is_3d() {
        return Err("the first-person template is 3D only".to_string());
    }
    let (mut project, library, _) = start(name, mode, PlayerPreset::FirstPerson3d, |scene| {
        if let Some(ground) = scene.actors.iter_mut().find(|a| a.name == "Ground") {
            ground.components.set_visual(Visual::Plane {
                color: "#3E4A5B".into(),
                size: [40.0, 40.0],
            });
        }
    })?;
    global(&mut project, "orbs", 0.0);
    global(&mut project, "escaped", 0.0);
    let scene = project.active_scene_mut();
    let wall = "#8D99AE";
    for (at, size) in [
        ([0.0, 1.5, 12.0], [24.0, 3.0, 0.5]),
        ([-12.0, 1.5, 0.0], [0.5, 3.0, 24.0]),
        ([12.0, 1.5, 0.0], [0.5, 3.0, 24.0]),
        // The far wall, with a doorway in the middle.
        ([-7.0, 1.5, -12.0], [10.0, 3.0, 0.5]),
        ([7.0, 1.5, -12.0], [10.0, 3.0, 0.5]),
    ] {
        place(
            scene,
            &library,
            actor_at("Wall", cuboid(wall, size), at),
            None,
            false,
        )?;
    }
    // Stairs up to a ledge.
    for i in 0..4 {
        let h = 0.25 * (i + 1) as f32;
        place(
            scene,
            &library,
            actor_at(
                "Step",
                cuboid("#6D6875", [2.0, h, 1.0]),
                [6.0, h / 2.0, 4.0 - i as f32],
            ),
            None,
            false,
        )?;
    }
    place(
        scene,
        &library,
        actor_at(
            "Ledge",
            cuboid("#6D6875", [4.0, 1.0, 4.0]),
            [6.0, 0.5, -1.5],
        ),
        None,
        false,
    )?;
    for at in [
        [0.0, 1.0, -4.0],
        [-8.0, 1.0, -6.0],
        [-8.0, 1.0, 8.0],
        [6.0, 1.8, -1.5],
    ] {
        let mut orb = actor_at(
            "Orb",
            Visual::Sphere {
                color: "#FFD166".into(),
                radius: 0.3,
            },
            at,
        );
        orb.graph.strands = vec![pickup_strand(
            "orbs",
            vec![K::If {
                condition: op(Op::Eq, vec![var("orbs"), num(4.0)]),
                body: body(vec![K::Broadcast {
                    name: "open".to_string(),
                }]),
            }],
        )];
        place(scene, &library, orb, None, true)?;
    }
    let mut door = actor_at(
        "Door",
        cuboid("#E4572E", [4.0, 3.0, 0.4]),
        [0.0, 1.5, -12.0],
    );
    door.graph.strands = vec![strand(
        0,
        vec![
            K::WhenMessage {
                name: "open".to_string(),
            },
            K::Glide {
                seconds: num(1.5),
                x: num(0.0),
                y: num(4.6),
                z: num(-12.0),
                easing: Default::default(),
            },
        ],
    )];
    place(scene, &library, door, Some(BodyType::Kinematic), false)?;
    let mut exit = actor_at(
        "Exit",
        cuboid("#06D6A0", [4.0, 2.0, 1.0]),
        [0.0, 1.0, -15.0],
    );
    exit.graph.strands = vec![strand(
        0,
        vec![
            K::WhenCollision {
                with: "Player".to_string(),
                phase: super::ContactPhase::Enter,
                scope: Default::default(),
            },
            K::SetVariable {
                name: "escaped".to_string(),
                value: num(1.0),
            },
            K::Say {
                text: Value::text("You escaped!"),
            },
        ],
    )];
    place(scene, &library, exit, None, true)?;
    Ok(finish(project, library))
}

/// Which cells a press on `cell` flips in the 3x3 puzzle.
pub fn lights_flipped(cell: usize) -> Vec<usize> {
    let (row, col) = (cell / 3, cell % 3);
    let mut out = vec![cell];
    if row > 0 {
        out.push(cell - 3);
    }
    if row < 2 {
        out.push(cell + 3);
    }
    if col > 0 {
        out.push(cell - 1);
    }
    if col < 2 {
        out.push(cell + 1);
    }
    out
}

/// The cells whose presses solve the starting board.
pub const LIGHTS_SOLUTION: [usize; 3] = [0, 4, 8];

/// A puzzle made of interface only: nine lights, each press flips a light and
/// its neighbours, and the board is solved when every light is on.
pub(super) fn ui_puzzle(name: &str, mode: Mode) -> Result<Project, String> {
    let mut project = Project::starter(name, mode);
    let scene = project.active_scene_mut();
    scene.actors.clear();
    scene.world.gravity = [0.0; 3];
    // Start from solved and undo the solution, so it is always solvable.
    let mut lights = [1.0; 9];
    for cell in LIGHTS_SOLUTION {
        for flipped in lights_flipped(cell) {
            lights[flipped] = 1.0 - lights[flipped];
        }
    }
    let cell = |i: usize| format!("cell {i}");
    let light = |i: usize| format!("light {i}");
    let mut strands = Vec::new();
    strands.push(strand(
        0,
        vec![
            K::WhenStarted,
            K::ShowLabel {
                element: Value::text("title"),
                text: Value::text("Turn every light on"),
                anchor: UiAnchor::Top,
                x: num(0.0),
                y: num(40.0),
                width: num(320.0),
                height: num(40.0),
                parent: Value::text(""),
            },
            K::Broadcast {
                name: "draw".to_string(),
            },
        ],
    ));
    for i in 0..9 {
        let mut kinds = vec![K::WhenUiClicked { element: cell(i) }];
        kinds.extend(lights_flipped(i).into_iter().map(|j| K::SetVariable {
            name: light(j),
            value: op(Op::Sub, vec![num(1.0), var(&light(j))]),
        }));
        kinds.push(K::ChangeVariable {
            name: "moves".to_string(),
            value: num(1.0),
        });
        kinds.push(K::Broadcast {
            name: "draw".to_string(),
        });
        strands.push(strand(320 * (1 + i as i32 % 3), kinds));
    }
    let button = |i: usize, label: &str| K::ShowButton {
        element: Value::text(cell(i)),
        label: Value::text(label),
        anchor: UiAnchor::Center,
        x: num(90.0 * (i % 3) as f64 - 90.0),
        y: num(90.0 * (i / 3) as f64 - 90.0),
        width: num(80.0),
        height: num(80.0),
        parent: Value::text(""),
    };
    let status = |text: Value| K::ShowLabel {
        element: Value::text("status"),
        text,
        anchor: UiAnchor::Bottom,
        x: num(0.0),
        y: num(-40.0),
        width: num(320.0),
        height: num(40.0),
        parent: Value::text(""),
    };
    let mut draw = vec![K::WhenMessage {
        name: "draw".to_string(),
    }];
    for i in 0..9 {
        draw.push(K::IfElse {
            condition: op(Op::Eq, vec![var(&light(i)), num(1.0)]),
            then_body: body(vec![button(i, "ON")]),
            else_body: body(vec![button(i, "off")]),
        });
    }
    let lit = (1..9).fold(var(&light(0)), |sum, i| {
        op(Op::Add, vec![sum, var(&light(i))])
    });
    draw.push(K::IfElse {
        condition: op(Op::Eq, vec![lit, num(9.0)]),
        then_body: body(vec![
            K::SetVariable {
                name: "solved".to_string(),
                value: num(1.0),
            },
            status(op(
                Op::Join,
                vec![
                    Value::text("Solved in "),
                    op(Op::Join, vec![var("moves"), Value::text(" moves")]),
                ],
            )),
        ]),
        else_body: body(vec![status(op(
            Op::Join,
            vec![Value::text("Moves: "), var("moves")],
        ))]),
    });
    strands.push(strand(1280, draw));
    let backdrop = if mode.is_3d() {
        cuboid("#2B2D42", [1.0, 1.0, 1.0])
    } else {
        rect("#2B2D42", 300.0, 300.0)
    };
    let mut board = Actor::new("Board", backdrop);
    board.graph.strands = strands;
    scene.add_actor(board);
    for (i, &on) in lights.iter().enumerate() {
        global(&mut project, &light(i), on);
    }
    global(&mut project, "moves", 0.0);
    global(&mut project, "solved", 0.0);
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lights_solution_solves_the_starting_board() {
        let mut lights = [1u8; 9];
        for cell in LIGHTS_SOLUTION {
            for f in lights_flipped(cell) {
                lights[f] ^= 1;
            }
        }
        assert_ne!(lights, [1; 9], "the board starts unsolved");
        for cell in LIGHTS_SOLUTION {
            for f in lights_flipped(cell) {
                lights[f] ^= 1;
            }
        }
        assert_eq!(lights, [1; 9]);
        assert_eq!(lights_flipped(4).len(), 5);
        assert_eq!(lights_flipped(0).len(), 3);
    }
}

/// Seats past the host's in the LAN co-op template: players 2 to 4.
const GUESTS: u8 = 3;

/// A top-down arena for up to four players over LAN. H hosts, J joins the
/// first game found; each guest gets the next player, who appears when they
/// join. Everyone collects the same gems.
pub(super) fn lan_co_op(name: &str, mode: Mode) -> Result<Project, String> {
    use crate::components::ActorComponent;
    if mode.is_3d() {
        return Err("the LAN co-op template is 2D only".to_string());
    }
    let (mut project, library, player) = start(name, mode, PlayerPreset::TopDown2d, |scene| {
        scene.actors.retain(|a| a.name != "Ground");
        scene.world.gravity = [0.0; 3];
        if let Some(player) = scene.actors.iter_mut().find(|a| a.name == "Player") {
            player.components.placement_mut().position = [-150.0, 0.0, 0.0];
        }
    })?;
    project.multiplayer.enabled = true;
    project.multiplayer.max_guests = usize::from(GUESTS);
    project.multiplayer.announce = true;
    global(&mut project, "gems", 0.0);
    // Each guest's pad: copies of every gameplay action for their player.
    let input = &mut project.settings.input;
    let bases: Vec<String> = input
        .actions
        .iter()
        .filter(|a| a.player == 0 && a.map == "Gameplay")
        .map(|a| a.name.clone())
        .collect();
    for seat in 1..=GUESTS {
        for base in &bases {
            if let Some(copy) = input.player_copy(base, seat)
                && input.find(&copy.name).is_none()
            {
                input.actions.push(copy);
            }
        }
    }
    let scene = project.active_scene_mut();
    let host = scene
        .actors
        .iter()
        .find(|a| a.id == player)
        .cloned()
        .ok_or("the player went missing")?;
    let colors = ["#FFD166", "#06D6A0", "#EF476F"];
    let mut names = vec![host.name.clone()];
    for seat in 1..=GUESTS {
        let mut guest = host.clone();
        guest.id = Actor::new("", rect("#FFFFFF", 1.0, 1.0)).id;
        guest.name = format!("Player {}", seat + 1);
        guest.refresh_physics_ids();
        guest.components.remove("Camera");
        guest.components.remove("PlayerCamera");
        for component in guest.components.iter_mut() {
            if let ActorComponent::CharacterMotor { motor } = component {
                motor.player = seat;
                motor.id = super::ids::ComponentId::generate();
            }
        }
        let tint = colors[usize::from(seat) - 1].to_string();
        let look = match guest.components.visual().cloned() {
            Some(Visual::Rect { size, .. }) => Visual::Rect { color: tint, size },
            Some(Visual::Circle { radius, .. }) => Visual::Circle {
                color: tint,
                radius,
            },
            Some(other) => other,
            None => rect(&tint, 40.0, 40.0),
        };
        guest.components.set_visual(look);
        let spawn = [-150.0 + 100.0 * f32::from(seat), 0.0, 0.0];
        guest.components.placement_mut().position = spawn;
        // Hidden and out of the way until someone plays them.
        guest.components.set_visible(false);
        let number = (seat + 1).to_string();
        guest.graph.strands = vec![
            strand(
                0,
                vec![
                    K::WhenPlayerJoins {
                        player: number.clone(),
                    },
                    K::GoTo {
                        x: num(f64::from(spawn[0])),
                        y: num(0.0),
                        z: num(0.0),
                    },
                    K::SetVisible { visible: true },
                ],
            ),
            strand(
                320,
                vec![
                    K::WhenPlayerLeaves { player: number },
                    K::SetVisible { visible: false },
                ],
            ),
        ];
        names.push(guest.name.clone());
        scene.add_actor(guest);
    }
    scene.add_actor(actor_at(
        "Floor",
        rect("#2B2D42", 760.0, 560.0),
        [0.0, 0.0, -1.0],
    ));
    for (at, size) in [
        ([0.0, 290.0], [780.0, 20.0]),
        ([0.0, -290.0], [780.0, 20.0]),
        ([-390.0, 0.0], [20.0, 600.0]),
        ([390.0, 0.0], [20.0, 600.0]),
    ] {
        place(
            scene,
            &library,
            actor_at(
                "Wall",
                rect("#8D99AE", size[0], size[1]),
                [at[0], at[1], 0.0],
            ),
            None,
            false,
        )?;
    }
    for (x, y) in [
        (220.0, 160.0),
        (-260.0, 200.0),
        (260.0, -200.0),
        (-240.0, -180.0),
    ] {
        let mut gem = actor_at(
            "Gem",
            Visual::Circle {
                color: "#4CC9F0".into(),
                radius: 10.0,
            },
            [x, y, 0.0],
        );
        // Whoever touches it first gets it for everyone.
        gem.graph.strands = names
            .iter()
            .enumerate()
            .map(|(i, who)| {
                let mut s = pickup_strand(
                    "gems",
                    vec![K::If {
                        condition: op(Op::Eq, vec![var("gems"), num(4.0)]),
                        body: body(vec![K::Broadcast {
                            name: "all gems".to_string(),
                        }]),
                    }],
                );
                s.x = 320 * i as i32;
                if let Some(first) = s.instructions.first_mut() {
                    first.kind = K::WhenCollision {
                        with: who.clone(),
                        phase: super::ContactPhase::Enter,
                        scope: Default::default(),
                    };
                }
                s
            })
            .collect();
        place(scene, &library, gem, None, true)?;
    }
    // The join screen: what to press, and who is here.
    let verb = |name: &str| {
        Value::op(
            Op::from_name(crate::verbs::OPERATOR),
            vec![Value::text(name)],
        )
    };
    let mut lobby = actor_at("Lobby", rect("#2B2D42", 1.0, 1.0), [0.0, 250.0, 1.0]);
    let say = |text: Value| K::Say { text };
    lobby.graph.strands = vec![
        strand(
            0,
            vec![
                K::WhenKeyPressed {
                    key: "h".to_string(),
                },
                K::If {
                    condition: op(Op::Not, vec![verb("hosting_lan")]),
                    body: body(vec![K::Verb {
                        verb: "host_lan".to_string(),
                        args: Vec::new(),
                    }]),
                },
            ],
        ),
        strand(
            320,
            vec![
                K::WhenKeyPressed {
                    key: "j".to_string(),
                },
                K::Verb {
                    verb: "join_lan".to_string(),
                    args: vec![Value::text("1")],
                },
            ],
        ),
        strand(
            640,
            vec![
                K::WhenStarted,
                K::Forever {
                    body: body(vec![K::IfElse {
                        condition: verb("hosting_lan"),
                        then_body: body(vec![say(op(
                            Op::Join,
                            vec![
                                Value::text("Hosting on the LAN. Players: "),
                                Value::op(Op::from_name("PlayerCount"), Vec::new()),
                            ],
                        ))]),
                        else_body: body(vec![K::IfElse {
                            condition: op(Op::Gt, vec![verb("lan_game_count"), num(0.0)]),
                            then_body: body(vec![say(op(
                                Op::Join,
                                vec![
                                    Value::text("J to join "),
                                    Value::op(
                                        Op::from_name(crate::verbs::OPERATOR),
                                        vec![Value::text("lan_game_name"), num(1.0)],
                                    ),
                                ],
                            ))]),
                            else_body: body(vec![say(Value::text(
                                "H to host a LAN game, J to join one",
                            ))]),
                        }]),
                    }]),
                },
            ],
        ),
    ];
    scene.add_actor(lobby);
    Ok(finish(project, library))
}
