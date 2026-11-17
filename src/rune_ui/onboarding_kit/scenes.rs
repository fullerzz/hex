//! The first run's scenes: the tour state a scene starts from, then the
//! first run played to a page.

use super::super::{Ui, onboarding};
use super::live::Page;

/// The tour state a Linux first-run scene starts from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::rune_ui) enum FirstRun {
    /// Nothing installed: the first run asks for the person's model file.
    Fresh,
    /// A chosen model file is a third of the way through its check.
    Downloading,
    /// The chosen model file couldn't be used.
    Failed,
    /// The model is installed.
    Ready,
    /// The model is installed, and the desktop asks before Hex's first
    /// paste (GNOME and KDE on Wayland).
    PasteAccess,
}

pub(in crate::rune_ui) fn first_run(ui: &mut Ui, state: FirstRun, page: Page) {
    *ui = super::super::portal_fixture();
    // Hex never starts a model by itself: only a chosen file is prepared.
    let chosen = state != FirstRun::Fresh;
    onboarding::fixture(ui, crate::desktop::onboarding::Step::Setup, chosen);
    match state {
        FirstRun::Fresh => {}
        FirstRun::Downloading => onboarding::set_downloaded(ui, 31),
        FirstRun::Failed => {
            if let Some(view) = &mut ui.view {
                view.transcription.preparing = None;
                view.transcription.error = Some("The connection was reset.".into());
            }
        }
        FirstRun::Ready | FirstRun::PasteAccess => {
            ui.picker.installed = Some(vec![ui.settings.transcription.model]);
            if let Some(view) = &mut ui.view {
                view.transcription.preparing = None;
            }
        }
    }
    if state == FirstRun::PasteAccess {
        ui.platform.paste = Some(super::super::paste_access::PasteAccess::Ask);
    }
    onboarding::poll(ui);
    super::live::fixture(ui, page);
}

pub(in crate::rune_ui) fn mac_first_run(ui: &mut Ui, step: onboarding::MacStep, page: Page) {
    *ui = super::super::mac_fixture();
    onboarding::mac_fixture(ui, step);
    // A fresh install's first page, before the person chose a file.
    if step == onboarding::MacStep::Setup
        && page == Page::Download
        && let Some(view) = &mut ui.view
    {
        view.transcription.preparing = None;
    }
    super::live::fixture(ui, page);
}
