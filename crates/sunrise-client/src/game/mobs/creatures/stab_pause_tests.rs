//! `LivingEntity.tick` stabs in `updatingUsingItem`, before `aiStep` moves the attacker: the
//! game settles a mob's stab at its `Pause::Stab`, so the push's slowdown
//! (`LivingEntity.causeExtraKnockback`) is in the move of the same tick.
use super::*;
use crate::game::Dim;
use crate::game::native_entity_refs::tests::with_game;
use sunrise_client::chunk::Chunk;
use sunrise_client::mobs::spear_use;

/// A zombie mid-charge, without goals, drifts 0.1 a tick in the air toward a NoAI villager 1.6
/// blocks ahead. The stab's tick moves it 0.6 of that (one push: the iron spear has no knockback),
/// where settling after the tick moved it the whole 0.1 and slowed it only for the next.
#[test]
#[ignore = "daily"]
fn a_mob_stab_slows_the_attacker_before_it_moves() {
    let root = std::env::temp_dir().join(format!(
        "sunrise-stab-pause-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    with_game(|game| {
        game.world_root = root.clone();
        game.enter_level(Dim::Overworld).unwrap();
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let types = game.res.mob_types();
        let (zombie, villager) = (
            types.get("minecraft:zombie").unwrap().clone(),
            types.get("minecraft:villager").unwrap().clone(),
        );
        let mut attacker = Mob::new(71, zombie, Vec3::new(0.5, 80.0, 0.5), 0.0);
        attacker
            .extra
            .set_main_hand_stack(Some(sunrise_client::items::ItemStack::new(
                "minecraft:iron_spear",
                1,
            )));
        assert!(spear_use::start_using_item(&mut attacker, false));
        attacker.extra.using_item = spear_use::main_hand_kinetic(&attacker)
            .expect("an iron spear")
            .delay_ticks
            + 1;
        attacker.previous = Vec3::new(0.5, 80.0, 0.4);
        attacker.velocity = Vec3::new(0.0, 0.0, 0.1);
        let mut target = Mob::new(72, villager, Vec3::new(0.5, 80.0, 2.1), 0.0);
        // No goals or targets: nothing adds to its motion but the drift.
        attacker.ai.goals = Default::default();
        attacker.ai.targets = Default::default();
        target.no_ai = true;
        for m in [&mut attacker, &mut target] {
            m.no_gravity = true;
            m.persistent = true;
        }
        game.mobs.push(attacker);
        game.mobs.push(target);
        game.tick_mobs().unwrap();
        let mob = |game: &Game<'_>, id| game.mobs.iter().find(|m| m.id == id).unwrap().clone();
        let (attacker, target) = (mob(game, 71), mob(game, 72));
        assert!(target.health < 20.0, "the stab hurt the villager");
        assert!(
            (attacker.position.z - (0.5 + 0.1 * 0.6)).abs() < 1e-9,
            "moved {} after the slowdown",
            attacker.position.z - 0.5
        );
    });
    let _ = std::fs::remove_dir_all(&root);
}
