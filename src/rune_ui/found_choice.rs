//! Onboarding's speech model list, shown only when the hub-cache scan found
//! a compatible model (`found_models`): the first run's model step
//! (`onboarding_kit::live`) shows one card of selectable rows: Hex's
//! recommended model first when its exact file is already here, then the
//! other models on this computer. Picking a row starts it at once, used where
//! it is. Nothing downloads: Hex's recommendation is offered only as a file
//! on this computer. Continue starts the selected row when none was picked.
//! Without a found model the step asks for the person's model file.

use super::Ui;
use crate::desktop::i18n::tr;
use crate::desktop::onboarding::{Input, ItemStatus, SetupItem};
use crate::transcription::found_models::Found;
use crate::transcription::models::{self as transcription_models, TranscriptionSelection};

/// Whether onboarding shows the list: something compatible was found and
/// the model isn't ready yet, or the person picked from it (it stays, so a
/// ready pick moves on from the list and Back returns to it).
pub(super) fn offered(ui: &Ui, model: ItemStatus) -> bool {
    !ui.found.models.is_empty() && (model != ItemStatus::Ready || ui.found.picked.is_some())
}

/// Whether the list waits for its first pick, so Continue starts the
/// selected row.
pub(super) fn pending(ui: &Ui, model: ItemStatus) -> bool {
    offered(ui, model)
        && matches!(model, ItemStatus::Todo | ItemStatus::Failed)
        && ui.found.picked.is_none()
        && !ui.found.adopting()
}

/// Hex's recommended model as the list first showed it: picking a found
/// model changes the saved selection, and the rows stay as they were.
fn recommended(ui: &Ui) -> TranscriptionSelection {
    ui.found
        .recommended
        .clone()
        .unwrap_or_else(|| ui.settings.transcription.clone())
}

/// One row of the list: Hex's recommended model's file, or another found
/// file.
enum Entry<'a> {
    Recommended,
    Found(&'a Found),
}

fn entries(ui: &Ui) -> Vec<Entry<'_>> {
    let recommended = recommended(ui).model;
    let on_disk = ui
        .found
        .models
        .iter()
        .any(|found| found.catalog == Some(recommended));
    on_disk
        .then_some(Entry::Recommended)
        .into_iter()
        .chain(
            ui.found
                .models
                .iter()
                .filter(|found| found.catalog != Some(recommended))
                .map(Entry::Found),
        )
        .collect()
}

/// One row of the list as the first run shows it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ChoiceRow {
    pub(super) name: String,
    pub(super) line: String,
    pub(super) recommended: bool,
    pub(super) selected: bool,
}

/// The list, one radio row per model.
pub(super) fn rows(ui: &Ui, model: ItemStatus) -> Vec<ChoiceRow> {
    let selected = ui.found.picked.unwrap_or(0);
    entries(ui)
        .into_iter()
        .enumerate()
        .map(|(index, entry)| row(ui, &entry, index == selected, model))
        .collect()
}

fn row(ui: &Ui, entry: &Entry<'_>, selected: bool, model: ItemStatus) -> ChoiceRow {
    let name = match entry {
        Entry::Recommended => transcription_models::definition(recommended(ui).model)
            .name
            .to_string(),
        Entry::Found(found) => found.catalog.map_or_else(
            || found.name.clone(),
            |id| transcription_models::definition(id).name.to_string(),
        ),
    };
    let started = selected && ui.found.picked.is_some();
    let line = match (entry, started, model) {
        (_, true, ItemStatus::Failed) => tr("Couldn't prepare this model").to_owned(),
        (_, true, ItemStatus::Ready) if !ui.found.adopting() => tr("Ready").to_owned(),
        (_, true, _) if ui.found.adopting() || model != ItemStatus::Ready => {
            tr("Checking…").to_owned()
        }
        (Entry::Recommended, ..) => tr("On this computer").to_owned(),
        (Entry::Found(found), ..) if found.catalog.is_some() => tr("On this computer").to_owned(),
        (Entry::Found(_), ..) => tr("On this computer · not tested by Hex").to_owned(),
    };
    ChoiceRow {
        name: name.to_owned(),
        line,
        recommended: matches!(entry, Entry::Recommended),
        selected,
    }
}

/// Whether the pick is under way or ready, so the first run can move on: a
/// model on this computer being checked, or checked and in use.
pub(super) fn started(ui: &Ui, model: ItemStatus) -> bool {
    ui.found.picked.is_some()
        && !ui.found.adopting()
        && matches!(model, ItemStatus::Ready | ItemStatus::Downloading(_))
}

/// Starts the selected row when the person continues without picking one.
pub(super) fn start_selected(ui: &mut Ui) {
    pick(ui, ui.found.picked.unwrap_or(0));
}

/// Picks row `index` and starts it. A newer pick supersedes one still
/// preparing, as the picker's does.
pub(super) fn pick(ui: &mut Ui, index: usize) {
    let failed = ui
        .tour
        .as_ref()
        .is_some_and(|tour| tour.flow.view().model == ItemStatus::Failed);
    if (ui.found.picked == Some(index) && !failed) || ui.found.adopting() {
        return;
    }
    let found = match entries(ui).get(index) {
        None => return,
        Some(Entry::Recommended) => None,
        Some(Entry::Found(found)) => Some((*found).clone()),
    };
    ui.found.picked = Some(index);
    ui.found.error = None;
    match found {
        None => prepare(ui, recommended(ui)),
        Some(found) => match found.catalog {
            Some(model) => {
                let language = transcription_models::definition(model)
                    .language_for(&recommended(ui).language)
                    .to_owned();
                prepare(ui, TranscriptionSelection { model, language });
            }
            // Hashed once and registered in place first, then prepared.
            None => super::found::adopt(ui, found, true),
        },
    }
}

/// Prepares `selection` for onboarding: through the tour the first time,
/// so its progress shows, and as a newer choice afterwards.
pub(super) fn prepare(ui: &mut Ui, selection: TranscriptionSelection) {
    let requested = ui.tour.as_ref().is_some_and(|tour| {
        !matches!(
            tour.flow.view().model,
            ItemStatus::Todo | ItemStatus::Failed
        )
    });
    if requested {
        ui.send(super::Request::ChooseModel {
            model: selection.model,
            language: selection.language,
        });
    } else {
        ui.found.choice = Some(selection);
        super::onboarding::input(ui, Input::Grant(SetupItem::Model));
    }
}
