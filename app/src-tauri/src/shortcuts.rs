//! Global shortcuts: which key combination triggers what, and moving the
//! registrations when the settings change.
//!
//! Registering is done through the small [`Registry`] trait so the logic —
//! in particular "never leave the user with half the shortcuts moved" — is
//! unit tested with a fake; the real implementation wraps the Tauri plugin.

use std::str::FromStr;

use tauri_plugin_global_shortcut::Shortcut;

use crate::settings::{clip_length_label, Settings};

/// What a global shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    /// Save the last this-many seconds.
    SaveClip(u64),
    ToggleMicMute,
}

/// What the user calls each shortcut, for messages.
fn name_of(action: ShortcutAction) -> String {
    match action {
        ShortcutAction::SaveClip(seconds) => {
            format!("guardar clip de {}", clip_length_label(seconds))
        }
        ShortcutAction::ToggleMicMute => "silenciar el micrófono".to_string(),
    }
}

/// The accelerators `settings` wants registered, in registration order.
fn wanted(settings: &Settings) -> Vec<(ShortcutAction, &str)> {
    let shortcuts = &settings.shortcuts;
    let mut list = vec![
        (
            ShortcutAction::SaveClip(shortcuts.clip_seconds),
            shortcuts.save_clip.as_str(),
        ),
        (
            ShortcutAction::ToggleMicMute,
            settings.microphone.mute_shortcut.as_str(),
        ),
    ];
    list.extend(shortcuts.extra_save_clip.iter().map(|extra| {
        (
            ShortcutAction::SaveClip(extra.seconds),
            extra.accelerator.as_str(),
        )
    }));
    list
}

/// The action a pressed shortcut triggers under `settings`, if any.
pub fn action_for(settings: &Settings, pressed: &Shortcut) -> Option<ShortcutAction> {
    wanted(settings).into_iter().find_map(|(action, text)| {
        Shortcut::from_str(text)
            .is_ok_and(|s| s == *pressed)
            .then_some(action)
    })
}

/// Where the OS shortcut registrations live.
pub trait Registry {
    fn register(&self, shortcut: Shortcut) -> Result<(), String>;
    fn unregister(&self, shortcut: Shortcut);
}

fn unavailable(action: ShortcutAction, accelerator: &str, why: &str) -> String {
    format!(
        "no se pudo registrar el atajo «{accelerator}» para {} (¿lo usa otra aplicación?): {why}",
        name_of(action)
    )
}

/// Register every shortcut in `settings` at startup. One that is taken or
/// invalid must not stop the app, so failures are returned (to tell the user)
/// instead of aborting.
pub fn register_all(registry: &impl Registry, settings: &Settings) -> Vec<String> {
    let mut problems = Vec::new();
    for (action, text) in wanted(settings) {
        match Shortcut::from_str(text) {
            Ok(shortcut) => {
                if let Err(e) = registry.register(shortcut) {
                    problems.push(unavailable(action, text, &e));
                }
            }
            Err(_) => problems.push(format!("atajo no válido: «{text}»")),
        }
    }
    problems
}

/// Move the registrations from what `old` had to what `new` wants. Either all
/// the changes happen or none do: if the OS refuses one, whatever was already
/// moved is put back and the error says which shortcut was the problem.
///
/// Registrations are compared as a set of key combinations, not by position:
/// the list of save shortcuts grows and shrinks, and a length that changes under
/// the same keys needs no OS call at all (the action is looked up when the key
/// is pressed).
pub fn switch(registry: &impl Registry, old: &Settings, new: &Settings) -> Result<(), String> {
    let mut wanted_now = Vec::new();
    for (action, text) in wanted(new) {
        let shortcut =
            Shortcut::from_str(text).map_err(|_| format!("atajo no válido: «{text}»"))?;
        wanted_now.push((action, text, shortcut));
    }
    let before: Vec<Shortcut> = wanted(old)
        .into_iter()
        .filter_map(|(_, text)| Shortcut::from_str(text).ok())
        .collect();

    let to_add: Vec<_> = wanted_now
        .iter()
        .filter(|(_, _, shortcut)| !before.contains(shortcut))
        .collect();
    let to_remove: Vec<Shortcut> = before
        .iter()
        .copied()
        .filter(|shortcut| !wanted_now.iter().any(|(_, _, now)| now == shortcut))
        .collect();

    // Free the old combinations first, so one shortcut can move onto the keys
    // another one is leaving (swapping the two, for instance).
    for shortcut in &to_remove {
        registry.unregister(*shortcut);
    }
    let mut registered = Vec::new();
    for (action, text, shortcut) in to_add {
        match registry.register(*shortcut) {
            Ok(()) => registered.push(*shortcut),
            Err(e) => {
                for shortcut in registered {
                    registry.unregister(shortcut);
                }
                for old in to_remove {
                    registry.register(old).ok();
                }
                return Err(unavailable(*action, text, &e));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ClipShortcut;
    use std::cell::RefCell;
    use std::collections::HashSet;

    fn key(text: &str) -> Shortcut {
        Shortcut::from_str(text).unwrap()
    }

    /// A registry that behaves like the OS: no duplicates, and some
    /// combinations belong to other applications.
    #[derive(Default)]
    struct Fake {
        registered: RefCell<HashSet<String>>,
        taken: Vec<&'static str>,
    }

    impl Fake {
        fn with(registered: &[&str], taken: &[&'static str]) -> Self {
            Self {
                registered: RefCell::new(registered.iter().map(|s| key(s).to_string()).collect()),
                taken: taken.to_vec(),
            }
        }

        fn has(&self, text: &str) -> bool {
            self.registered.borrow().contains(&key(text).to_string())
        }

        fn count(&self) -> usize {
            self.registered.borrow().len()
        }
    }

    impl Registry for Fake {
        fn register(&self, shortcut: Shortcut) -> Result<(), String> {
            if self.taken.iter().any(|t| key(t) == shortcut) {
                return Err("otra aplicación lo tiene".into());
            }
            if !self.registered.borrow_mut().insert(shortcut.to_string()) {
                return Err("ya está registrado".into());
            }
            Ok(())
        }

        fn unregister(&self, shortcut: Shortcut) {
            self.registered.borrow_mut().remove(&shortcut.to_string());
        }
    }

    fn settings(save: &str, mute: &str) -> Settings {
        let mut s = Settings::default();
        s.shortcuts.save_clip = save.into();
        s.microphone.mute_shortcut = mute.into();
        s
    }

    /// `settings(SAVE, MUTE)` plus these extra save shortcuts.
    fn with_extras(extras: &[(&str, u64)]) -> Settings {
        let mut s = settings(SAVE, MUTE);
        s.shortcuts.extra_save_clip = extras
            .iter()
            .map(|(accelerator, seconds)| ClipShortcut {
                accelerator: (*accelerator).into(),
                seconds: *seconds,
            })
            .collect();
        s
    }

    const SAVE: &str = "CommandOrControl+Shift+S";
    const MUTE: &str = "CommandOrControl+Shift+M";

    #[test]
    fn a_pressed_shortcut_maps_to_its_action() {
        let s = settings("Alt+F9", MUTE);
        assert_eq!(
            action_for(&s, &key("Alt+F9")),
            Some(ShortcutAction::SaveClip(30))
        );
        assert_eq!(
            action_for(&s, &key(MUTE)),
            Some(ShortcutAction::ToggleMicMute)
        );
        // The old default no longer saves clips.
        assert_eq!(action_for(&s, &key(SAVE)), None);
    }

    #[test]
    fn spelling_does_not_matter() {
        let s = settings("CmdOrCtrl+Shift+S", MUTE);
        assert_eq!(
            action_for(&s, &key(SAVE)),
            Some(ShortcutAction::SaveClip(30))
        );
        assert_eq!(
            action_for(&s, &key("Shift+CommandOrControl+S")),
            Some(ShortcutAction::SaveClip(30))
        );
    }

    #[test]
    fn an_unparseable_setting_matches_nothing() {
        let s = settings("nonsense", MUTE);
        assert_eq!(action_for(&s, &key("Alt+F9")), None);
        assert_eq!(
            action_for(&s, &key(MUTE)),
            Some(ShortcutAction::ToggleMicMute)
        );
    }

    #[test]
    fn startup_registers_both() {
        let registry = Fake::default();
        assert!(register_all(&registry, &settings(SAVE, MUTE)).is_empty());
        assert!(registry.has(SAVE) && registry.has(MUTE));
    }

    #[test]
    fn a_taken_shortcut_is_reported_but_does_not_block_the_others() {
        let registry = Fake::with(&[], &[SAVE]);
        let problems = register_all(&registry, &settings(SAVE, MUTE));
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("guardar clip"), "{}", problems[0]);
        assert!(problems[0].contains(SAVE));
        assert!(registry.has(MUTE), "the other shortcut still works");
    }

    #[test]
    fn startup_reports_an_unparseable_shortcut_too() {
        let registry = Fake::default();
        let problems = register_all(&registry, &settings("nonsense", MUTE));
        assert_eq!(problems.len(), 1);
        assert!(problems[0].contains("no válido"));
        assert!(registry.has(MUTE));
    }

    #[test]
    fn changing_the_save_shortcut_moves_only_that_registration() {
        let registry = Fake::with(&[SAVE, MUTE], &[]);
        switch(&registry, &settings(SAVE, MUTE), &settings("Alt+F9", MUTE)).unwrap();
        assert!(!registry.has(SAVE));
        assert!(registry.has("Alt+F9") && registry.has(MUTE));
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn nothing_changed_means_nothing_is_touched() {
        let registry = Fake::with(&[SAVE, MUTE], &[]);
        switch(&registry, &settings(SAVE, MUTE), &settings(SAVE, MUTE)).unwrap();
        assert!(registry.has(SAVE) && registry.has(MUTE));
        // Spelling the same keys differently is not a change either.
        switch(
            &registry,
            &settings(SAVE, MUTE),
            &settings("CmdOrCtrl+Shift+S", MUTE),
        )
        .unwrap();
        assert!(registry.has(SAVE) && registry.has(MUTE));
    }

    #[test]
    fn a_shortcut_can_move_onto_the_keys_another_one_is_leaving() {
        let registry = Fake::with(&[SAVE, MUTE], &[]);
        // Swap them.
        switch(&registry, &settings(SAVE, MUTE), &settings(MUTE, SAVE)).unwrap();
        assert!(registry.has(SAVE) && registry.has(MUTE));
        assert_eq!(registry.count(), 2);

        // Or chain: save takes mute's keys while mute goes elsewhere.
        let registry = Fake::with(&[SAVE, MUTE], &[]);
        switch(&registry, &settings(SAVE, MUTE), &settings(MUTE, "Alt+F9")).unwrap();
        assert!(!registry.has(SAVE));
        assert!(registry.has(MUTE) && registry.has("Alt+F9"));
    }

    #[test]
    fn a_refused_shortcut_leaves_everything_as_it_was() {
        // Both change; the second one is taken by another application.
        let registry = Fake::with(&[SAVE, MUTE], &["Alt+F10"]);
        let err = switch(
            &registry,
            &settings(SAVE, MUTE),
            &settings("Alt+F9", "Alt+F10"),
        )
        .unwrap_err();
        assert!(
            err.contains("Alt+F10") && err.contains("silenciar"),
            "{err}"
        );
        assert!(
            registry.has(SAVE) && registry.has(MUTE),
            "old ones restored"
        );
        assert!(!registry.has("Alt+F9"), "the new one was rolled back");
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn a_refused_first_shortcut_restores_the_old_one() {
        let registry = Fake::with(&[SAVE, MUTE], &["Alt+F9"]);
        let err = switch(&registry, &settings(SAVE, MUTE), &settings("Alt+F9", MUTE)).unwrap_err();
        assert!(err.contains("guardar clip"), "{err}");
        assert!(registry.has(SAVE) && registry.has(MUTE));
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn an_unparseable_target_is_refused_before_touching_anything() {
        let registry = Fake::with(&[SAVE, MUTE], &[]);
        let err = switch(
            &registry,
            &settings(SAVE, MUTE),
            &settings("nonsense", MUTE),
        )
        .unwrap_err();
        assert!(err.contains("no válido"));
        assert!(registry.has(SAVE) && registry.has(MUTE));
    }

    #[test]
    fn recovers_when_the_old_shortcut_was_never_registered() {
        // E.g. it had been taken at startup: moving away from it must work.
        let registry = Fake::with(&[MUTE], &[]);
        switch(&registry, &settings(SAVE, MUTE), &settings("Alt+F9", MUTE)).unwrap();
        assert!(registry.has("Alt+F9") && registry.has(MUTE));
    }

    // ---- save shortcuts with their own length ----

    #[test]
    fn each_save_shortcut_saves_its_own_length() {
        let mut s = with_extras(&[("F9", 15), ("Alt+F9", 120)]);
        s.shortcuts.clip_seconds = 45;
        assert_eq!(
            action_for(&s, &key(SAVE)),
            Some(ShortcutAction::SaveClip(45))
        );
        assert_eq!(
            action_for(&s, &key("F9")),
            Some(ShortcutAction::SaveClip(15))
        );
        assert_eq!(
            action_for(&s, &key("Alt+F9")),
            Some(ShortcutAction::SaveClip(120))
        );
        assert_eq!(
            action_for(&s, &key(MUTE)),
            Some(ShortcutAction::ToggleMicMute)
        );
        assert_eq!(action_for(&s, &key("F10")), None);
    }

    #[test]
    fn startup_registers_the_extra_shortcuts_too() {
        let registry = Fake::default();
        let problems = register_all(&registry, &with_extras(&[("F9", 15), ("F10", 60)]));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(registry.count(), 4);
        assert!(registry.has("F9") && registry.has("F10"));
    }

    #[test]
    fn a_taken_extra_shortcut_is_reported_by_its_length() {
        let registry = Fake::with(&[], &["F9"]);
        let problems = register_all(&registry, &with_extras(&[("F9", 90), ("F10", 15)]));
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].contains("guardar clip de 1 min 30 s") && problems[0].contains("F9"),
            "{}",
            problems[0]
        );
        assert!(registry.has("F10"), "the others still work");
    }

    #[test]
    fn adding_and_removing_extra_shortcuts_moves_only_those() {
        let registry = Fake::with(&[SAVE, MUTE], &[]);
        let none = with_extras(&[]);
        let two = with_extras(&[("F9", 15), ("F10", 60)]);

        switch(&registry, &none, &two).unwrap();
        assert_eq!(registry.count(), 4);
        assert!(registry.has("F9") && registry.has("F10"));

        switch(&registry, &two, &with_extras(&[("F10", 60)])).unwrap();
        assert!(!registry.has("F9") && registry.has("F10"));
        assert!(registry.has(SAVE) && registry.has(MUTE));
        assert_eq!(registry.count(), 3);

        switch(&registry, &with_extras(&[("F10", 60)]), &none).unwrap();
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn changing_only_a_length_does_not_touch_the_os() {
        // The fake refuses a second registration of the same keys: any attempt to
        // re-register F9 would fail the switch.
        let registry = Fake::with(&[SAVE, MUTE, "F9"], &[]);
        switch(
            &registry,
            &with_extras(&[("F9", 15)]),
            &with_extras(&[("F9", 120)]),
        )
        .unwrap();
        assert!(registry.has("F9"));
        assert_eq!(registry.count(), 3);

        // The main length too.
        let mut longer = with_extras(&[]);
        longer.shortcuts.clip_seconds = 90;
        switch(&registry, &with_extras(&[("F9", 120)]), &longer).unwrap();
        assert!(registry.has(SAVE));
        assert!(!registry.has("F9"), "only the extra that went away");
    }

    #[test]
    fn an_extra_shortcut_can_move_onto_the_keys_the_main_one_leaves() {
        let registry = Fake::with(&[SAVE, MUTE], &[]);
        let before = with_extras(&[]);
        let mut after = with_extras(&[(SAVE, 15)]);
        after.shortcuts.save_clip = "Alt+F9".into();
        switch(&registry, &before, &after).unwrap();
        assert!(registry.has(SAVE) && registry.has("Alt+F9") && registry.has(MUTE));
        assert_eq!(registry.count(), 3);
    }

    #[test]
    fn a_refused_extra_shortcut_puts_everything_back() {
        // Three change at once; the last one is taken by another application.
        let registry = Fake::with(&[SAVE, MUTE, "F9"], &["F11"]);
        let err = switch(
            &registry,
            &with_extras(&[("F9", 15)]),
            &with_extras(&[("F10", 15), ("F11", 60)]),
        )
        .unwrap_err();
        assert!(err.contains("F11") && err.contains("1 min"), "{err}");
        assert!(registry.has("F9"), "the one that was removed is back");
        assert!(!registry.has("F10"), "the one that was added is gone");
        assert!(registry.has(SAVE) && registry.has(MUTE));
        assert_eq!(registry.count(), 3);
    }

    #[test]
    fn an_unparseable_extra_is_refused_before_touching_anything() {
        let registry = Fake::with(&[SAVE, MUTE, "F9"], &[]);
        let err = switch(
            &registry,
            &with_extras(&[("F9", 15)]),
            &with_extras(&[("nonsense", 15)]),
        )
        .unwrap_err();
        assert!(err.contains("no válido"));
        assert!(registry.has("F9") && registry.count() == 3);
    }
}
