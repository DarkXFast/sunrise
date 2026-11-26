//! `LivingEntity.tick` stabs in `updatingUsingItem`, before `aiStep` moves the attacker: the
//! game settles a mob's stab at its `Pause::Stab`, so the push's slowdown
//! (`LivingEntity.causeExtraKnockback`) is in the move of the same tick.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use crate::game::{Dim, in_own_process};
use sunrise_client::chunk::Chunk;
use sunrise_client::mobs::spear_use;

/// A server game with one empty, entity-ticking chunk at the origin.
fn with_stab_world(test: impl FnOnce(&mut Game<'_>)) {
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
        test(game);
    });
    let _ = std::fs::remove_dir_all(&root);
}

/// A persistent mob of `kind` floating without gravity at `at`.
fn floating(game: &Game<'_>, id: u32, kind: &str, at: Vec3) -> Mob {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    let mut m = Mob::new(id, kind, at, 0.0);
    m.no_gravity = true;
    m.persistent = true;
    m
}

/// Zombie 71 at (0.5, 80, 0.5), past its iron spear's delay and drifting `speed` a tick along +z
/// in the air, without goals or targets: nothing adds to its motion but the drift.
fn charging_zombie(game: &Game<'_>, speed: f64) -> Mob {
    let mut attacker = floating(game, 71, "minecraft:zombie", Vec3::new(0.5, 80.0, 0.5));
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
    attacker.previous = Vec3::new(0.5, 80.0, 0.5 - speed);
    attacker.velocity = Vec3::new(0.0, 0.0, speed);
    attacker.ai.goals = Default::default();
    attacker.ai.targets = Default::default();
    attacker
}

/// A zombie mid-charge drifts 0.1 a tick in the air toward a NoAI villager 1.6 blocks ahead. The
/// stab's tick moves it 0.6 of that (one push: the iron spear has no knockback), where settling
/// after the tick moved it the whole 0.1 and slowed it only for the next.
#[test]
#[ignore = "daily"]
fn a_mob_stab_slows_the_attacker_before_it_moves() {
    in_own_process(
        "game::creatures::borrowed_tick::stab_pause_tests::a_mob_stab_slows_the_attacker_before_it_moves",
        || {
            with_stab_world(|game| {
                let attacker = charging_zombie(game, 0.1);
                let mut target =
                    floating(game, 72, "minecraft:villager", Vec3::new(0.5, 80.0, 2.1));
                target.no_ai = true;
                game.mobs.push(attacker);
                game.mobs.push(target);
                game.tick_mobs().unwrap();
                let mob =
                    |game: &Game<'_>, id| game.mobs.iter().find(|m| m.id == id).unwrap().clone();
                let (attacker, target) = (mob(game, 71), mob(game, 72));
                assert!(target.health < 20.0, "the stab hurt the villager");
                assert!(
                    (attacker.position.z - (0.5 + 0.1 * 0.6)).abs() < 1e-9,
                    "moved {} after the slowdown",
                    attacker.position.z - 0.5
                );
            });
        },
    );
}

/// Hears a stab's dismount of villager 72 and the travels of zombie 71, and discards the zombie
/// from that dismount's `entity_mount` when `discard`.
struct DiscardOnDismount {
    discard: bool,
    heard: Arc<std::sync::Mutex<Vec<&'static str>>>,
}

#[mod_api::__subscriptions]
impl mod_api::Hooks for DiscardOnDismount {
    fn entity_mount(
        &self,
        host: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::EntityMountEvent<'_>,
    ) -> mod_api::Flow {
        if !event.mounting && event.entity == 72 {
            let heard = if !self.discard {
                "dismount"
            } else if host
                .entity(71)
                .expect("the attacker is live at its stab")
                .discard(host)
            {
                "dismount, attacker discarded"
            } else {
                "dismount, discard refused"
            };
            self.heard.lock().unwrap().push(heard);
        }
        mod_api::Flow::Continue
    }

    fn living_travel(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::LivingTravelEvent<'_>,
    ) -> mod_api::Flow {
        if event.entity == 71 {
            self.heard.lock().unwrap().push("attacker travels");
        }
        mod_api::Flow::Continue
    }
}

/// `LivingEntity.tick` runs `aiStep` only `if (!isRemoved())`. Through the game's mob pass, a
/// zombie charging at 0.15 a tick stabs and unseats a villager riding a pig: the dismount's
/// `entity_mount` comes before the zombie travels, and a listener that discards the zombie there
/// ends its tick at the stab, with no travel after it, and the zombie leaves the level.
#[test]
#[ignore = "daily"]
fn a_mob_discarded_while_its_stab_is_settled_does_not_travel() {
    in_own_process(
        "game::creatures::borrowed_tick::stab_pause_tests::a_mob_discarded_while_its_stab_is_settled_does_not_travel",
        || {
            for (discard, heard) in [
                (false, ["dismount", "attacker travels"].as_slice()),
                (true, ["dismount, attacker discarded"].as_slice()),
            ] {
                let log = Arc::new(std::sync::Mutex::new(Vec::new()));
                let mods = vec![mod_api::ModEntry::new(
                    "stab-discard-test",
                    Box::new(DiscardOnDismount {
                        discard,
                        heard: log.clone(),
                    }),
                )];
                mod_platform::testing::with_mods(mods.into(), || {
                    with_stab_world(|game| discard_on_dismount_case(game, discard));
                });
                assert_eq!(*log.lock().unwrap(), heard, "discard {discard}");
            }
        },
    );
}

/// Villager 72 rides pig 73 1.6 blocks ahead of zombie 71, charging at 0.15 a tick: fast enough
/// for the iron spear's dismount. One mob pass stabs and unseats it.
fn discard_on_dismount_case(game: &mut Game<'_>, discard: bool) {
    let attacker = charging_zombie(game, 0.15);
    let mut target = floating(game, 72, "minecraft:villager", Vec3::new(0.5, 80.0, 2.1));
    let mut pig = floating(game, 73, "minecraft:pig", Vec3::new(0.5, 80.0, 2.1));
    target.no_ai = true;
    pig.no_ai = true;
    game.mobs.push(attacker);
    game.mobs.push(target);
    game.mobs.push(pig);
    assert!(game.try_mount_mob(72, 73));
    let seated = game.mob_object_mut(72).unwrap();
    seated.previous = seated.position;
    game.tick_mobs().unwrap();
    let target = game.mob_object(72).unwrap();
    assert!(target.health < 20.0, "the stab hurt the villager");
    assert_eq!(target.extra.mount, None, "the stab unseated the villager");
    assert_eq!(
        game.mobs.iter().any(|m| m.id == 71),
        !discard,
        "a discarded attacker leaves the level"
    );
}

fn stab_on(target: Who) -> MobEvent {
    use sunrise_client::mobs::{Damage, WeaponAttack, WeaponHit};
    MobEvent::WeaponAttack(WeaponHit {
        target,
        source: Damage::mob(71, [0.5, 80.0, 0.5]),
        weapon: sunrise_client::items::ItemStack::new("minecraft:iron_spear", 1),
        amount: 4.0,
        knockback: 0.0,
        direction: [0.0, 1.0],
        attack: WeaponAttack::Stab {
            deals_damage: true,
            deals_knockback: true,
            dismounts: false,
        },
    })
}

fn stab_memory(target: Who, contact: usize) -> MobEvent {
    MobEvent::StabMemory {
        attacker: 71,
        target,
        contact,
        attempted_damage: true,
        otherwise_affected: true,
    }
}

fn delivered(deliveries: Vec<StabDelivery>) -> Vec<String> {
    deliveries
        .into_iter()
        .map(|delivery| {
            let (to, event) = match delivery {
                StabDelivery::Host(event) => ("host".to_owned(), event),
                StabDelivery::Guest(conn, mob, event) => (format!("guest {conn} by {mob}"), event),
            };
            let what = match event {
                MobEvent::WeaponAttack(hit) => format!("stab {:?}", hit.target),
                MobEvent::StabMemory {
                    target, contact, ..
                } => format!("memory {target:?} {contact}"),
                MobEvent::EntityEvent { event, .. } => format!("event {event}"),
                other => format!("{other:?}"),
            };
            format!("{to}: {what}")
        })
        .collect()
}

/// `KineticWeapon.damageEntities` finishes each contact's `stabAttack` before the next and
/// broadcasts event 2 last: a guest met first (tagged with its connection, as
/// `MobWorld::route_player_event` tags it) is settled before the mob met after it, not after it.
/// An untagged player stab goes to the guest the tick sees, or to the host when it sees none.
#[test]
fn a_stab_settles_a_guest_contact_before_a_later_mob_contact() {
    let tagged = |event| MobEvent::ForPlayer {
        conn: Some(12),
        event: Box::new(event),
    };
    let broadcast = MobEvent::EntityEvent {
        mob: 71,
        event: 2,
        at: [0.5, 80.0, 0.5],
    };
    let stabs = vec![
        tagged(stab_on(Who::Player)),
        tagged(stab_memory(Who::Player, 0)),
        stab_on(Who::Mob(72)),
        stab_memory(Who::Mob(72), 1),
        broadcast.clone(),
    ];
    assert_eq!(
        delivered(route_stab_events(stabs, None, &[(12, 12)], 71)),
        [
            "guest 12 by 71: stab Player",
            "guest 12 by 71: memory Player 0",
            "host: stab Mob(72)",
            "host: memory Mob(72) 1",
            "host: event 2",
        ]
    );
    for (seen_guest, to) in [(Some(13), "guest 13 by 71"), (None, "host")] {
        let stabs = vec![
            stab_on(Who::Player),
            stab_on(Who::Mob(72)),
            broadcast.clone(),
        ];
        assert_eq!(
            delivered(route_stab_events(stabs, seen_guest, &[], 71)),
            [
                format!("{to}: stab Player"),
                "host: stab Mob(72)".to_owned(),
                "host: event 2".to_owned(),
            ]
        );
    }
}

fn player_view(x: f64, uuid: i32) -> PlayerView {
    PlayerView {
        position: Vec3::new(x, 80.0, 0.0),
        bounds: sunrise_core::geometry::Aabb::new(
            Vec3::new(x - 0.3, 80.0, -0.3),
            Vec3::new(x + 0.3, 81.8, 0.3),
        ),
        eye: [x, 81.62, 0.0],
        alive: true,
        invulnerable: false,
        creative: false,
        sneaking: false,
        swimming: false,
        look: [0.0; 3],
        main_hand: None,
        off_hand: None,
        head: None,
        chest: None,
        legs: None,
        feet: None,
        armor_cover: 0.0,
        invisible: false,
        spectator: false,
        gold_armor: false,
        hurt_by: None,
        last_hurt: None,
        velocity: Vec3::ZERO,
        yaw: 0.0,
        sprinting: false,
        riding: false,
        hero: false,
        effects: Vec::new(),
        uuid: [uuid, 0, 0, 0],
        allies: Default::default(),
        remote: true,
        sleeping: false,
        sleep_timer: 0,
        passenger: false,
        riding_vehicle: None,
        vehicle: None,
    }
}

/// A stab that kills the guest the tick sees: the pause's callback ends the views the tick held,
/// so its saved player selection is refreshed by identity before it resumes, as after a hurt or
/// an equipment event. A dead guest has no live view (`lan_guest_views` leaves it out) and none
/// is lent to the resumed tick, so the selection drops it; the host and another guest still in
/// play keep their places with their live views. No equipment pause comes between: the stab's
/// own refresh does it.
#[test]
fn a_stab_pause_drops_the_selected_guest_its_stab_killed() {
    assert!(refreshes_selection(Some(&Pause::Stab { events: 3 })));
    assert!(refreshes_selection(Some(&Pause::Equipment {
        first: sunrise_core::equipment_slot::EquipmentSlot::MainHand,
    })));
    assert!(!refreshes_selection(None));
    let host = player_view(1.0, 1);
    let tick_player = |entity_id, connection: Option<u32>, view: &PlayerView| TickPlayer {
        entity_id,
        connection,
        view: view.clone(),
    };
    let mut selection = PlayerSelection::new(
        vec![
            tick_player(1, None, &host),
            tick_player(12, Some(12), &player_view(12.0, 12)),
            tick_player(13, Some(13), &player_view(13.0, 13)),
        ],
        Some(12),
    )
    .unwrap();
    let identity = |conn: u32| Some(([conn as i32, 0, 0, 0], conn));
    let host_seat = (Some(&host), Some(([1, 0, 0, 0], 1)));
    // The guest the tick sees (12) is dead after the stab: only guest 13 has a view.
    let live = vec![(13, player_view(14.0, 13))];
    let slots = [(13, 0)].into_iter().collect();
    selection.refresh_views(|player| {
        live_selection_view(
            player,
            (Some(12), None, identity(12)),
            &live,
            &slots,
            identity,
            host_seat,
        )
    });
    assert!(selection.selected().is_none(), "the dead guest is dropped");
    assert!(selection.get(12).is_none());
    assert_eq!(selection.get(13).unwrap().view.position.x, 14.0);
    assert_eq!(selection.get(1).unwrap().view.position.x, 1.0);
    // A tick that sees the host: the host's view is the one lent to it. A guest whose
    // connection now names another entity (a rejoin) is not the player saved.
    let rejoined = |conn: u32| Some(([conn as i32, 0, 0, 0], conn + 100));
    selection.refresh_views(|player| {
        live_selection_view(
            player,
            (None, host_seat.0, host_seat.1),
            &live,
            &slots,
            rejoined,
            (None, None),
        )
    });
    assert!(selection.get(13).is_none());
    assert_eq!(selection.get(1).unwrap().view.position.x, 1.0);
}
