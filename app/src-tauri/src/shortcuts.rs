//! Global shortcuts: which key combination triggers what, and moving the
//! registrations when the settings change.
//!
//! Registering is done through the small [`Registry`] trait so the logic —
//! in particular "never leave the user with half the shortcuts moved" — is
//! unit tested with a fake; the real implementation wraps the Tauri plugin.

use std::str::FromStr;

use tauri_plugin_global_shortcut::Shortcut;

use crate::settings::Settings;

/// What a global shortcut does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutAction {
    SaveClip,
    ToggleMicMute,
}

/// What the user calls each shortcut, for messages.
fn name_of(action: ShortcutAction) -> &'static str {
    match action {
        ShortcutAction::SaveClip => "guardar clip",
        ShortcutAction::ToggleMicMute => "silenciar el micrófono",
    }
}

/// The accelerators `settings` wants registered, in registration order.
fn wanted(settings: &Settings) -> [(ShortcutAction, &str); 2] {
    [
        (ShortcutAction::SaveClip, &settings.shortcuts.save_clip),
        (
            ShortcutAction::ToggleMicMute,
            &settings.microphone.mute_shortcut,
        ),
    ]
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
pub fn switch(registry: &impl Registry, old: &Settings, new: &Settings) -> Result<(), String> {
    struct Change {
        action: ShortcutAction,
        text: String,
        old: Option<Shortcut>,
        new: Shortcut,
    }
    let mut changes = Vec::new();
    for ((action, old_text), (_, new_text)) in wanted(old).into_iter().zip(wanted(new)) {
        let new_shortcut =
            Shortcut::from_str(new_text).map_err(|_| format!("atajo no válido: «{new_text}»"))?;
        let old_shortcut = Shortcut::from_str(old_text).ok();
        if old_shortcut != Some(new_shortcut) {
            changes.push(Change {
                action,
                text: new_text.to_string(),
                old: old_shortcut,
                new: new_shortcut,
            });
        }
    }

    // Free the old combinations first, so one shortcut can move onto the keys
    // another one is leaving (swapping the two, for instance).
    for change in &changes {
        if let Some(old) = change.old {
            registry.unregister(old);
        }
    }
    let mut registered = Vec::new();
    for change in &changes {
        match registry.register(change.new) {
            Ok(()) => registered.push(change.new),
            Err(e) => {
                for shortcut in registered {
                    registry.unregister(shortcut);
                }
                for old in changes.iter().filter_map(|c| c.old) {
                    registry.register(old).ok();
                }
                return Err(unavailable(change.action, &change.text, &e));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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

    const SAVE: &str = "CommandOrControl+Shift+S";
    const MUTE: &str = "CommandOrControl+Shift+M";

    #[test]
    fn a_pressed_shortcut_maps_to_its_action() {
        let s = settings("Alt+F9", MUTE);
        assert_eq!(
            action_for(&s, &key("Alt+F9")),
            Some(ShortcutAction::SaveClip)
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
        assert_eq!(action_for(&s, &key(SAVE)), Some(ShortcutAction::SaveClip));
        assert_eq!(
            action_for(&s, &key("Shift+CommandOrControl+S")),
            Some(ShortcutAction::SaveClip)
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
}
