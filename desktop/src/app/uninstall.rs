//! Uninstalling (Swift `Support.uninstall`), after the person confirmed it in Settings: sign out,
//! stop the engine, remove the hooks (restoring the agents' settings), Lantern's support files,
//! settings and desktop integrations, then the app itself, and quit. Agents and sessions Lantern
//! started keep running. When something can't be removed, the person decides whether to remove
//! the app anyway (`system.uninstall` is `Partial`).
use super::Lantern;
use crate::model::Uninstall;
use crate::os::Event;
use std::path::PathBuf;

impl Lantern {
    /// ConfirmUninstall.
    pub(super) fn uninstall(&mut self) {
        if self.model.system.uninstall == Uninstall::Removing {
            return;
        }
        super::trace!("uninstall: removing");
        self.model.system.uninstall = Uninstall::Removing;
        self.installed = self.model.prefs.bundle.clone().map(PathBuf::from);
        self.sign_out();
        // The engine holds files in Lantern's data folder open (Windows can't delete those).
        self.engine.stop();
        crate::os::task(self.tx.clone(), || Ok(crate::os::uninstall_parts()), |result| Event::Uninstalled(result.unwrap_or_default()));
    }

    pub(super) fn uninstalled(&mut self, failures: Vec<String>) {
        super::trace!("uninstall: {} problem(s)", failures.len());
        if failures.is_empty() {
            self.remove_app();
        } else {
            self.model.system.uninstall = Uninstall::Partial { failures };
        }
    }

    /// The last step (also "Move App to Trash Anyway"): remove the app, then quit.
    pub(super) fn remove_app(&mut self) {
        match crate::os::remove_app(self.installed.as_deref()) {
            Ok(()) => self.windows.quit(),
            Err(message) => self.model.system.uninstall = Uninstall::AppFailed { message },
        }
    }

    /// "Keep the App" / OK: Lantern keeps running without what was removed, as on the Mac.
    pub(super) fn dismiss_uninstall(&mut self) {
        if self.model.system.uninstall != Uninstall::Removing {
            self.model.system.uninstall = Uninstall::Idle;
            // Uninstalling stopped the engine; Lantern keeps working without what was removed.
            if !self.connected && !cfg!(test) {
                self.restart_engine();
            }
        }
    }

    /// Remembers where Lantern is installed, so uninstalling can remove that folder even after an
    /// update runs from Lantern's data folder.
    pub(super) fn note_install_folder(&mut self) {
        let Some(folder) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(PathBuf::from)) else { return };
        if crate::os::installed_bundle(&folder) && !folder.starts_with(crate::settings::data_dir()) {
            let folder = folder.to_string_lossy().into_owned();
            if self.model.prefs.bundle.as_deref() != Some(&folder) {
                self.model.prefs.bundle = Some(folder);
                self.save_prefs();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lantern() -> Lantern {
        let (engine, _) = crate::sidecar::test_sender();
        Lantern::new(engine, None)
    }

    #[test]
    fn a_partial_uninstall_waits_for_the_persons_choice() {
        let mut lantern = lantern();
        lantern.model.system.uninstall = Uninstall::Removing;
        lantern.uninstalled(vec!["Removing the hooks: engine missing.".into()]);
        assert_eq!(lantern.model.system.uninstall, Uninstall::Partial { failures: vec!["Removing the hooks: engine missing.".into()] });
        assert!(!lantern.windows.quitting(), "nothing more happens until they choose");
        lantern.dismiss_uninstall();
        assert_eq!(lantern.model.system.uninstall, Uninstall::Idle);
    }

    #[test]
    fn removing_the_app_from_a_build_in_the_repo_is_refused_and_lantern_keeps_running() {
        // Tests run from cargo's target folder: never an installed bundle, so nothing is trashed.
        let mut lantern = lantern();
        lantern.model.system.uninstall = Uninstall::Partial { failures: vec!["x".into()] };
        lantern.remove_app();
        assert!(matches!(lantern.model.system.uninstall, Uninstall::AppFailed { .. }), "{:?}", lantern.model.system.uninstall);
        assert!(!lantern.windows.quitting());
    }

    #[test]
    fn a_second_confirm_while_removing_does_nothing_and_settings_stay_unsaved() {
        let mut lantern = lantern();
        lantern.model.system.uninstall = Uninstall::Removing;
        lantern.uninstall();
        assert_eq!(lantern.model.system.uninstall, Uninstall::Removing);
        lantern.dismiss_uninstall();
        assert_eq!(lantern.model.system.uninstall, Uninstall::Removing, "can't dismiss while removing");
    }
}
