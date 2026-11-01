//! `patch #Name { … }`: an inheriting scene patching its base's descendants, found by name.

use std::path::Path;

use bevy_app::App;
use bevy_asset::{AssetServer, Handle};
use bevy_ecs::{
    entity::Entity,
    hierarchy::Children,
    lifecycle::Insert,
    name::Name,
    observer::On,
    resource::Resource,
    system::ResMut,
    world::World,
};
use bevy_scene::{PatchedComponents, ScenePatch, WorldSceneExt};
use core::any::TypeId;

use crate::{
    test_support::memory_asset_app,
    tests::{register_fixtures, scene, Marker, Position},
    write_scene_text, WriteSettings,
};

/// An app serving `files`, with each of them loaded (and so resolved) before it returns. The
/// handles keep them loaded.
fn app_with(files: &[(&str, &str)]) -> (App, Vec<Handle<ScenePatch>>) {
    let (mut app, dir) = memory_asset_app();
    register_fixtures(&mut app);
    app.finish();
    app.cleanup();
    for (path, source) in files {
        dir.insert_asset_text(Path::new(path), source);
    }
    let asset_server = app.world().resource::<AssetServer>().clone();
    let handles: Vec<_> = files
        .iter()
        .map(|(path, _)| asset_server.load::<ScenePatch>(path.to_string()))
        .collect();
    for frame in 0..10_000 {
        app.update();
        if frame >= 100 {
            std::thread::sleep(core::time::Duration::from_millis(1));
        }
        if handles.iter().all(|handle| asset_server.is_loaded(handle)) {
            break;
        }
    }
    assert!(
        handles.iter().all(|handle| asset_server.is_loaded(handle)),
        "the scene assets never finished loading"
    );
    (app, handles)
}

/// Every entity below `root` named `name`.
fn find(world: &World, root: Entity, name: &str) -> Vec<Entity> {
    let mut found = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter());
        }
        if entity != root && world.get::<Name>(entity).is_some_and(|n| n.as_str() == name) {
            found.push(entity);
        }
    }
    found
}

fn one(world: &World, root: Entity, name: &str) -> Entity {
    let found = find(world, root, name);
    assert_eq!(found.len(), 1, "expected one {name} below the root");
    found[0]
}

#[test]
fn a_patch_block_patches_the_named_descendant_instead_of_adding_one() {
    let (mut app, _handles) = app_with(&[(
        "a.bsn",
        "Children [ #X Position { x: 1.0, y: 2.0 } -- #Y Marker ]",
    )]);
    let b = scene(&app, "b.bsn", ":\"a.bsn\"\npatch #X { Position { y: 5.0 } }");
    let world = app.world_mut();
    let root = world.spawn_scene(b).unwrap().id();

    assert_eq!(world.get::<Children>(root).unwrap().len(), 2, "nothing was added");
    let x = one(world, root, "X");
    // A partial patch keeps the fields the base gives it.
    assert_eq!(world.get::<Position>(x), Some(&Position { x: 1.0, y: 5.0, z: 0.0 }));
    assert!(world
        .get::<PatchedComponents>(x)
        .is_some_and(|patched| patched.0.contains(&TypeId::of::<Position>())));
    let y = one(world, root, "Y");
    assert!(world.get::<PatchedComponents>(y).is_none());
}

#[test]
fn a_patch_block_finds_a_descendant_at_any_depth_by_its_name_component() {
    let (mut app, _handles) = app_with(&[(
        "a.bsn",
        "Children [ #Wall Children [ bevy_ecs::name::Name(\"Wall.Glass\") Position { z: 4.0 } ] ]",
    )]);
    let b = scene(&app, "b.bsn", ":\"a.bsn\"\npatch #\"Wall.Glass\" { Marker }");
    let world = app.world_mut();
    let root = world.spawn_scene(b).unwrap().id();

    let glass = one(world, root, "Wall.Glass");
    assert!(world.get::<Marker>(glass).is_some());
    assert_eq!(world.get::<Position>(glass), Some(&Position { x: 0.0, y: 0.0, z: 4.0 }));
}

#[test]
fn the_patched_component_is_inserted_once() {
    #[derive(Resource, Default)]
    struct Inserts(usize);

    let (mut app, _handles) = app_with(&[("a.bsn", "Children [ #X Position { x: 1.0 } ]")]);
    let b = scene(&app, "b.bsn", ":\"a.bsn\"\npatch #X { Position { y: 5.0 } }");
    let world = app.world_mut();
    world.init_resource::<Inserts>();
    world.add_observer(|_: On<Insert<Position>>, mut inserts: ResMut<Inserts>| {
        inserts.0 += 1;
    });
    world.spawn_scene(b).unwrap();
    assert_eq!(world.resource::<Inserts>().0, 1);
}

#[test]
fn a_nested_patch_block_tells_apart_a_name_the_base_repeats() {
    let (mut app, _handles) = app_with(&[(
        "a.bsn",
        "Children [ #A Children [ #Light Position ] -- #B Children [ #Light Position ] ]",
    )]);

    let ambiguous = scene(&app, "b.bsn", ":\"a.bsn\"\npatch #Light { Marker }");
    let Err(error) = app.world_mut().spawn_scene(ambiguous) else {
        panic!("a name two descendants carry must not resolve");
    };
    let message = format!("{error}");
    assert!(message.contains("matches 2 descendants"), "{message}");

    let nested = scene(
        &app,
        "c.bsn",
        ":\"a.bsn\"\npatch #B { patch #Light { Position { x: 3.0 } } }",
    );
    let world = app.world_mut();
    let root = world.spawn_scene(nested).unwrap().id();
    let a = one(world, root, "A");
    let b = one(world, root, "B");
    let in_a = one(world, a, "Light");
    let in_b = one(world, b, "Light");
    assert_eq!(world.get::<Position>(in_a), Some(&Position::default()));
    assert_eq!(world.get::<Position>(in_b), Some(&Position { x: 3.0, y: 0.0, z: 0.0 }));
}

#[test]
fn a_name_that_matches_nothing_names_the_scene_the_base_and_the_name() {
    let (mut app, _handles) = app_with(&[("a.bsn", "Children [ #X ]")]);
    let b = scene(&app, "levels/b.bsn", ":\"a.bsn\"\npatch #X { patch #Nope { Marker } }");
    let Err(error) = app.world_mut().spawn_scene(b) else {
        panic!("a patch naming no descendant must not resolve");
    };
    let message = format!("{error}");
    assert!(message.contains("levels/b.bsn"), "{message}");
    assert!(message.contains("\"a.bsn\""), "{message}");
    assert!(message.contains("#\"X\" > #\"Nope\""), "{message}");
    assert!(message.contains("matches no descendant"), "{message}");
}

#[test]
fn a_patch_block_cannot_rename_its_descendant() {
    let (app, _handles) = app_with(&[("a.bsn", "Children [ #X ]")]);
    let document = bevy_bsn::parse(":\"a.bsn\"\npatch #X { bevy_ecs::name::Name(\"Y\") }").unwrap();
    let registry = app
        .world()
        .resource::<bevy_ecs::reflect::AppTypeRegistry>()
        .clone();
    assert!(matches!(
        crate::DynamicScene::from_document(&document, "b.bsn", &registry),
        Err(crate::DynamicSceneBuildError::PatchRenames { .. })
    ));
}

#[test]
fn an_outer_patch_lands_on_an_inner_one() {
    // The house's room patches the room's X; the level patches it again, field by field.
    let (mut app, _handles) = app_with(&[
        ("room.bsn", "Children [ #X Position { z: 1.0 } ]"),
        (
            "house.bsn",
            "Children [ :\"room.bsn\" #R patch #X { Position { x: 7.0 } } ]",
        ),
    ]);
    let level = scene(&app, "level.bsn", ":\"house.bsn\"\npatch #X { Position { y: 9.0 } }");
    let world = app.world_mut();
    let root = world.spawn_scene(level).unwrap().id();
    let x = one(world, root, "X");
    assert_eq!(world.get::<Position>(x), Some(&Position { x: 7.0, y: 9.0, z: 1.0 }));
}

#[test]
fn a_patch_reaches_into_a_base_child_that_has_a_base_of_its_own() {
    let (mut app, _handles) = app_with(&[
        ("door.bsn", "Children [ #Pane Position { z: 2.0 } ]"),
        ("room.bsn", "Children [ :\"door.bsn\" #Door ]"),
    ]);
    let level = scene(&app, "level.bsn", ":\"room.bsn\"\npatch #Pane { Marker }");
    let world = app.world_mut();
    let root = world.spawn_scene(level).unwrap().id();
    let pane = one(world, root, "Pane");
    assert!(world.get::<Marker>(pane).is_some());
    assert_eq!(world.get::<Position>(pane), Some(&Position { x: 0.0, y: 0.0, z: 2.0 }));
}

#[test]
fn an_outer_patch_records_what_both_patches_set() {
    let (mut app, _handles) = app_with(&[
        ("room.bsn", "Children [ #X ]"),
        ("house.bsn", "Children [ :\"room.bsn\" #R patch #X { Marker } ]"),
    ]);
    let level = scene(&app, "level.bsn", ":\"house.bsn\"\npatch #X { Position { y: 9.0 } }");
    let world = app.world_mut();
    let root = world.spawn_scene(level).unwrap().id();
    let x = one(world, root, "X");
    let patched = &world.get::<PatchedComponents>(x).expect("X is patched").0;
    assert!(patched.contains(&TypeId::of::<Marker>()), "{patched:?}");
    assert!(patched.contains(&TypeId::of::<Position>()), "{patched:?}");
}

#[test]
fn a_base_below_that_did_not_resolve_is_named_instead_of_missing_the_name() {
    // door.bsn fails to resolve (its own patch names nothing), so what it spawns is unknown.
    let (mut app, _handles) = app_with(&[
        ("knob.bsn", "Children [ #Knob ]"),
        ("door.bsn", ":\"knob.bsn\"\npatch #Nothing { Marker }"),
        ("room.bsn", "Children [ :\"door.bsn\" #Door ]"),
    ]);
    let level = scene(&app, "levels/b.bsn", ":\"room.bsn\"\npatch #Knob { Marker }");
    let Err(error) = app.world_mut().spawn_scene(level) else {
        panic!("a patch searching below an unresolved base must not resolve");
    };
    let message = format!("{error}");
    assert!(message.contains("levels/b.bsn"), "{message}");
    assert!(message.contains("door.bsn"), "{message}");
    assert!(!message.contains("matches no descendant"), "{message}");
}

/// The names of the entities below `root`, in pre-order.
fn spawned_names(world: &World, root: Entity) -> Vec<String> {
    let mut names = Vec::new();
    let mut stack: Vec<Entity> = world
        .get::<Children>(root)
        .map(|c| c.iter().copied().rev().collect())
        .unwrap_or_default();
    while let Some(entity) = stack.pop() {
        names.push(world.get::<Name>(entity).map_or(String::new(), |n| n.as_str().to_string()));
        if let Some(children) = world.get::<Children>(entity) {
            stack.extend(children.iter().rev());
        }
    }
    names
}

#[test]
fn the_resolve_time_walk_sees_the_tree_that_spawns() {
    // `InheritedEntity` mirrors `ResolvedScene::apply_with` by hand: what the patches are found
    // in has to be what is spawned, a base's children before the entity's own.
    let (mut app, handles) = app_with(&[
        ("door.bsn", "Children [ #Pane ]"),
        ("room.bsn", "Children [ #X -- :\"door.bsn\" #D ]"),
        (
            "level.bsn",
            "Children [ :\"room.bsn\" #R patch #Pane { Marker } Children [ #Own ] -- #Side Children [ #Leaf ] ]",
        ),
    ]);
    let walked = {
        let patches = app.world().resource::<bevy_asset::Assets<ScenePatch>>();
        let level = &patches.get(&handles[2]).unwrap().resolved.as_ref().unwrap().scene;
        let mut walked = Vec::new();
        bevy_scene::InheritedEntity::root(level, patches).visit_descendants(&mut |entity| {
            walked.push(entity.name().map_or(String::new(), |n| n.as_str().to_string()));
        });
        walked
    };
    let instance = scene(&app, "instance.bsn", ":\"level.bsn\"");
    let world = app.world_mut();
    let root = world.spawn_scene(instance).unwrap().id();
    assert_eq!(walked, ["R", "X", "D", "Pane", "Own", "Side", "Leaf"]);
    assert_eq!(walked, spawned_names(world, root));
    assert!(world.get::<Marker>(one(world, root, "Pane")).is_some());
}

// ---------------------------------------------------------------------------------------
// Writing back
// ---------------------------------------------------------------------------------------

fn write(world: &mut World, root: Entity) -> String {
    write_scene_text(world, root, &WriteSettings::default()).expect("writes")
}

#[test]
fn a_patched_descendant_writes_back_as_a_patch_block() {
    let (mut app, _handles) = app_with(&[(
        "a.bsn",
        "Children [ #X Position { x: 1.0, y: 2.0 } -- #Y Marker ]",
    )]);
    let b = scene(&app, "b.bsn", ":\"a.bsn\"\npatch #X { Position { y: 5.0 } }");
    let root = app.world_mut().spawn_scene(b).unwrap().id();
    let text = write(app.world_mut(), root);
    assert_eq!(text, ":\"a.bsn\"\npatch #X {\n    bevy_bsn_asset::tests::Position { y: 5.0 }\n}\n");

    // What it wrote loads back into the same entities.
    let again = scene(&app, "c.bsn", &text);
    let world = app.world_mut();
    let copy = world.spawn_scene(again).unwrap().id();
    let x = one(world, copy, "X");
    assert_eq!(world.get::<Position>(x), Some(&Position { x: 1.0, y: 5.0, z: 0.0 }));
}

#[test]
fn only_what_a_patch_block_set_and_still_differs_is_written_back() {
    let (mut app, _handles) = app_with(&[("a.bsn", "Children [ #X Position { x: 1.0 } -- #Y ]")]);
    let b = scene(&app, "b.bsn", ":\"a.bsn\"\npatch #X { Position { y: 5.0 } }");
    let root = app.world_mut().spawn_scene(b).unwrap().id();
    let world = app.world_mut();
    let y = one(world, root, "Y");
    // Play state on a descendant nobody patched is not authored.
    world.entity_mut(y).insert(Marker);
    let text = write(world, root);
    assert!(!text.contains("patch #Y"), "{text}");

    // Put back to what the base gives it, the patch has nothing left to say.
    let x = one(world, root, "X");
    world.get_mut::<Position>(x).unwrap().y = 0.0;
    let text = write(world, root);
    assert_eq!(text, ":\"a.bsn\"\n");
}

#[test]
fn a_repeated_name_writes_back_nested_under_an_ancestor_that_tells_it_apart() {
    let (mut app, _handles) = app_with(&[(
        "a.bsn",
        "Children [ #A Children [ #Light Position ] -- #B Children [ #Light Position ] ]",
    )]);
    let b = scene(
        &app,
        "b.bsn",
        ":\"a.bsn\"\npatch #B { patch #Light { Position { x: 3.0 } } }",
    );
    let root = app.world_mut().spawn_scene(b).unwrap().id();
    let text = write(app.world_mut(), root);
    assert_eq!(
        text,
        ":\"a.bsn\"\npatch #B {\n    patch #Light {\n        bevy_bsn_asset::tests::Position { x: 3.0 }\n    }\n}\n"
    );
}

#[test]
fn a_base_s_own_patch_is_not_written_into_the_scene_that_inherits_it() {
    let (mut app, _handles) = app_with(&[
        ("room.bsn", "Children [ #X Position { z: 1.0 } ]"),
        (
            "house.bsn",
            "Children [ :\"room.bsn\" #R patch #X { Position { x: 7.0 } } ]",
        ),
    ]);
    let level = scene(&app, "level.bsn", ":\"house.bsn\"\npatch #X { Position { y: 9.0 } }");
    let root = app.world_mut().spawn_scene(level).unwrap().id();
    let text = write(app.world_mut(), root);
    assert_eq!(
        text,
        ":\"house.bsn\"\npatch #X {\n    bevy_bsn_asset::tests::Position { y: 9.0 }\n}\n"
    );
}

#[test]
fn a_despawned_sibling_does_not_shift_what_is_written_back() {
    let (mut app, _handles) = app_with(&[("a.bsn", "Children [ #W1 -- #W2 -- #X ]")]);
    let b = scene(&app, "b.bsn", ":\"a.bsn\"\npatch #X { Position { y: 5.0 } }");
    let root = app.world_mut().spawn_scene(b).unwrap().id();
    let world = app.world_mut();
    let w1 = one(world, root, "W1");
    world.despawn(w1);
    let text = write(world, root);
    assert_eq!(text, ":\"a.bsn\"\npatch #X {\n    bevy_bsn_asset::tests::Position { y: 5.0 }\n}\n");
}
