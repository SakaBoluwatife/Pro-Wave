#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Ovrtone desktop app. Pure Rust (egui/eframe) for Windows, Linux and macOS.
// Debug builds abort if the audio callback allocates (see `ovrtone-engine` output.rs).
#[cfg(debug_assertions)]
#[global_allocator]
static ALLOC_GUARD: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

mod aaf;
mod adm;
mod alternatives;
mod arrangement_track;
mod audio_encode;
mod audio_file_editor;
mod audio_rec;
mod audio_tags;
mod audio_track_editor;
mod automation;
mod autosave;
mod beat_store;
mod bench;
mod bounce;
mod bus_plugins;
mod channel_rack;
mod chord_track;
mod clip_launcher;
mod coming_from;
mod control_surface;
mod dock;
mod engine_sync;
mod event_list;
mod feedback;
mod file_browser;
mod file_commands;
mod flex_edit;
mod fx_ui;
mod gain_staging;
mod graphics;
mod groove;
mod groove_template;
mod groups;
mod hardware;
mod hrtf;
mod i18n;
mod icons;
mod import;
mod inspector;
mod key_commands;
mod left_panel;
mod license;
mod marker_track;
mod mastering;
mod menus;
mod midi_file;
mod midi_fx;
mod midi_fx_ui;
mod midi_in;
mod midi_learn;
mod midi_out;
mod midi_routing;
mod mixer;
mod model;
mod mtc_in;
mod native_menu;
mod note_edit;
mod note_tools;
mod pattern;
#[cfg(test)]
mod perf;
mod performance;
mod piano_roll;
mod plugin_favourites;
mod plugin_guard;
mod plugin_presets;
mod plugin_rack;
mod plugin_scan;
mod plugin_trust;
mod project_history;
mod project_io;
mod project_thumbs;
mod quick_dialogs;
mod recent_files;
mod recorder;
mod region_edit;
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod reliability_tests;
mod routing_matrix;
mod sample_browser;
mod sample_place;
mod sample_tags;
mod sampler_window;
mod score;
mod screensets;
mod scripter;
mod session_import;
mod session_organizer;
mod settings;
mod side_panes;
mod signature_track;
mod smart_controls;
mod smart_tempo;
mod song_pattern;
mod spatial_export;
mod spatial_window;
mod speaker_setup;
mod start_screen;
mod stem_splitter;
mod stem_worker;
mod step_editor;
mod strip_silence;
mod sync_menu;
mod takes;
mod tempo_track;
mod theme;
mod themes;
mod track_icons;
mod tracks_area;
mod transform;
mod transform_ui;
mod transport;
mod typing;
mod ui_scale;
#[cfg(test)]
mod ui_test;
mod undo;
mod tuner;
mod updater;
mod widgets;
mod window_chrome;

use model::Project;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Bottom {
    Mixer,
    PianoRoll,
}

pub struct UiState {
    pub show_mixer: bool,
    pub show_library: bool,
    /// The control bar's display shows everything, not only bar, beat, tempo, signature and key.
    pub lcd_full: bool,
    /// The display when not Custom: 0 beats and project, 1 beats and time, 2 beats, 3 time.
    pub lcd_mode: u8,
    pub show_inspector: bool,
    /// List Editors (the Event list on the right).
    pub show_lists: bool,
    pub lists: event_list::ListView,
    /// The MIDI recording in progress.
    pub rec: recorder::Recorder,
    /// Recordings on the other armed tracks, while armed tracks listen to their own MIDI devices.
    pub rec_more: std::collections::BTreeMap<usize, recorder::Recorder>,
    /// The audio recording in progress, and the open audio input.
    pub audio_rec: audio_rec::AudioRecorder,
    /// Capture Recording was asked for.
    pub capture_cmd: bool,
    /// The disk underruns already told of.
    pub underruns_told: u32,
    /// Record Repeat or New Take was picked, and how far going back to record again has got.
    pub rec_cmd: Option<takes::RecCmd>,
    pub rec_restart: Option<takes::Restart>,
    pub show_quick_help: bool,
    pub show_toolbar: bool,
    /// The entries of the Toolbar, in order.
    pub toolbar: Vec<&'static str>,
    /// The mode buttons after the control bar display.
    pub header_modes: Vec<transport::HeaderMode>,
    /// A track whose ARA plugin (Melodyne) is to be asked for its notes, to make a MIDI track.
    pub ara_midi: Option<usize>,
    /// A spatial bus plugin whose window is to open.
    pub bus_plugin_editor: Option<(u64, String)>,
    /// An export for the beat store under way (File > Export > For Beat Store).
    pub beat_store: Option<beat_store::Pending>,
    /// The For Beat Store window: which stores open after the export.
    pub beat_store_dialog: Option<beat_store::Dialog>,
    /// The "coming from" window: which program the user knows.
    pub coming_from: coming_from::ComingFrom,
    /// The clip launcher window (F10).
    pub launcher: clip_launcher::LauncherView,
    /// Help > Send Feedback.
    pub feedback: feedback::FeedbackView,
    /// The audio device went away: when it was last looked for.
    pub audio_lost: Option<std::time::Instant>,
    /// Playing at the last frame, so loudness can start again when playback starts.
    pub mastering_was_playing: bool,
    /// The Mackie Control surface's MIDI port, as chosen in Settings.
    pub surface_port: Option<String>,
    /// The surface speaks HUI rather than Mackie Control.
    pub surface_hui: bool,
    /// The Note Pad.
    pub show_notes: bool,
    /// The Loop Browser and All Files.
    pub browser: file_browser::BrowserView,
    /// Which tab of the left panel is showing: Library, Browser or Plugins.
    pub left_tab: left_panel::LeftTab,
    /// The Browser tab: sample folders, search, favorites and the preview.
    pub samples: sample_browser::SampleBrowser,
    /// The Plugins tab: the VST3 plugins found on this computer.
    pub plugins: left_panel::PluginList,
    /// Files to add to the project, from the browsers.
    pub open_paths: Vec<std::path::PathBuf>,
    /// Help > Check for Updates… was chosen.
    pub update_check: bool,
    /// Help > Check for Updates Automatically.
    pub update_auto: bool,
    pub bottom: Bottom,
    /// Last import error or engine warning, shown in the menu bar.
    pub status: Option<String>,
    pub tracks: tracks_area::TracksView,
    pub mixer: mixer::MixerView,
    /// What Mute and Solo were last frame, so mix groups follow (see `groups.rs`).
    pub group_watch: groups::GroupWatch,
    pub inspector: inspector::InspectorView,
    pub piano: piano_roll::PianoView,
    /// The MIDI FX window of the selected track is open.
    pub show_midi_fx: bool,
    /// The MIDI Transform window.
    pub transform: transform_ui::TransformView,
    /// The editor panel shows the Score instead of the Piano Roll for ordinary MIDI regions.
    pub score_tab: bool,
    pub score: score::ScoreView,
    /// The screenset called back or saved last, for its window.
    pub screenset_on: Option<u8>,
    /// A note the Score just added is sounding: its pitch and when it stops.
    pub score_heard: Option<(u8, f64)>,
    /// Flex Time and Flex Pitch work running in the background.
    pub flex: flex_edit::FlexState,
    /// The Audio File Editor window (edits that change a file itself).
    pub file_edit: audio_file_editor::FileEditor,
    /// The audio Groove Track's hits being read.
    pub groove: groove::GrooveState,
    /// An audio region being read to make a groove template.
    pub groove_make: groove_template::MakeState,
    /// File > Import > MIDI File… or Export > Selection as MIDI File…, run after the menus.
    pub midi_cmd: Option<midi_file::MidiCmd>,
    /// The Pattern and Drummer editors, which take the Piano Roll's place for those regions.
    pub steps: step_editor::StepView,
    /// True while the pointer is over the Piano Roll (it owns Delete/Cmd+R then).
    pub piano_focus: bool,
    /// Live notes from the Piano Roll, forwarded to the engine every frame.
    pub audition: Vec<piano_roll::Audition>,
    /// Bounce and export: the window, and what is being rendered.
    pub bounce: bounce::BounceState,
    pub rack: channel_rack::RackView,
    pub settings: settings::SettingsView,
    pub dialogs: quick_dialogs::QuickDialogs,
    pub smart: smart_controls::SmartControls,
    pub midi_learn: midi_learn::MidiLearn,
    pub stems: stem_splitter::StemSplitter,
    pub spatial: spatial_window::SpatialView,
    pub gain_staging: gain_staging::GainStagingView,
    pub organizer: session_organizer::OrganizerView,
    pub performance: performance::PerformanceView,
    /// Where each panel is docked.
    pub layout: dock::Layout,
    /// While the plugins of a project just opened load: the most that were left.
    pub opening_plugins: Option<usize>,
    /// When the plugin load screen came up: after a while it can be clicked away.
    pub opening_since: Option<std::time::Instant>,
    pub strip_silence: strip_silence::StripSilence,
    pub keys: key_commands::KeyMap,
    /// What the steps of Undo and Redo were, for the Undo History.
    pub undo_labels: Vec<String>,
    pub redo_labels: Vec<String>,
    pub mastering: mastering::MasteringView,
    /// The control-bar tuner: pitch detection on the stereo output.
    pub tuner: tuner::Tuner,
    /// A sample let go over the Tracks area, waiting for what it should become.
    pub sample_drop: Option<sample_place::SampleDrop>,
    /// The computer keyboard as a piano, and what it played this frame.
    pub typing: typing::Typing,
    pub typed: Vec<recorder::LiveNote>,
    /// File menu command picked this frame; run by `App` once the panels are drawn.
    pub file_cmd: Option<file_commands::FileCmd>,
    pub file_task: Option<file_commands::FileTask>,
    pub window_cmd: Option<menus::WindowCmd>,
    /// A New / Open waiting for the answer to the "save changes?" prompt.
    /// The launch screen is up, following the plugin scan.
    pub launching: bool,
    pub ask_save: Option<file_commands::FileCmd>,
    /// A `.prowave` file was just saved: ask whether to move to the new extension.
    pub offer_new_extension: bool,
    /// The old file whose owner answered no, not to ask at every save.
    pub keep_old_extension: Option<std::path::PathBuf>,
    /// True while the "revert to the last saved version?" prompt is showing.
    pub ask_revert: bool,
    /// The project picked in File > Open Recent, until `App` has opened it.
    pub recent_pick: Option<std::path::PathBuf>,
    /// File > Open Recent. In memory only until `main` loads the saved list.
    pub recent: recent_files::Recent,
    /// DSP load shown by the LCD's CPU meter (1.0 = full budget); peak-held, falls slowly.
    pub cpu_load: f32,
    /// How busy reading audio files from disk is, 0..=1, for the HD meter.
    pub disk_load: f32,
    /// When a note was last played in from a MIDI or computer keyboard: render-ahead waits for
    /// a quiet spell after it.
    pub live_note_at: Option<std::time::Instant>,
    /// What opening another DAW's session left behind, to show once.
    pub import_log: Option<(String, Vec<String>)>,
    /// Minutes between crash-recovery copies (0: never), and backups kept on save.
    pub autosave_minutes: u32,
    pub backups_kept: usize,
    /// The Project History window (File > Project History).
    pub history_open: bool,
    /// A project that would not open, and its newest backup, offered instead.
    pub damaged: Option<(std::path::PathBuf, std::path::PathBuf, String)>,
    /// Light (white) menu bar and menus, toggled by the Smart Controls button.
    pub light_menus: bool,
    /// The Project Chooser, while it covers the window (File > New from Template…).
    pub chooser: Option<start_screen::Chooser>,
    /// What the chooser's panel says for the project about to start.
    pub new_details: Option<start_screen::Details>,
    /// The audio devices the chooser found.
    pub chooser_devices: start_screen::FoundDevices,
    /// A project file to take a picture for, once this many frames have been drawn.
    pub thumbnail_wanted: Option<(std::path::PathBuf, u32)>,
    /// Frames to keep drawing after asking for a picture: it only arrives on a later frame.
    pub thumbnail_settle: u32,
    /// The sample holder whose window is open, by its plugin's number.
    pub sampler_window: Option<u64>,
    pub sampler_view: sampler_window::SamplerView,
    /// The user's interface size (1.0 = 100 %), on top of fitting small windows (`ui_scale`).
    pub ui_scale: f32,
    /// Edit > Undo / Redo picked this frame; run by `App` (which keeps the history).
    pub undo_cmd: Option<undo::UndoCmd>,
    /// Whether there is something to undo / redo (greys the Edit entries).
    pub can_undo: bool,
    pub can_redo: bool,
}

impl UiState {
    fn new() -> Self {
        Self {
            show_mixer: false,
            show_library: false,
            lcd_full: transport::lcd_default() == Some(transport::CUSTOM_LCD),
            lcd_mode: transport::lcd_default()
                .filter(|m| *m != transport::CUSTOM_LCD)
                .unwrap_or(0),
            show_inspector: true,
            show_lists: false,
            lists: event_list::ListView::default(),
            rec: recorder::Recorder::default(),
            rec_more: std::collections::BTreeMap::new(),
            audio_rec: audio_rec::AudioRecorder::default(),
            capture_cmd: false,
            underruns_told: 0,
            rec_cmd: None,
            rec_restart: None,
            show_quick_help: false,
            show_toolbar: false,
            toolbar: side_panes::load_toolbar(),
            header_modes: vec![
                transport::HeaderMode::Typing,
                transport::HeaderMode::LowLatency,
            ],
            ara_midi: None,
            bus_plugin_editor: None,
            beat_store: None,
            beat_store_dialog: None,
            coming_from: coming_from::ComingFrom::default(),
            launcher: clip_launcher::LauncherView::default(),
            feedback: feedback::FeedbackView::default(),
            audio_lost: None,
            mastering_was_playing: false,
            surface_port: control_surface::chosen(),
            surface_hui: control_surface::hui_chosen(),
            show_notes: false,
            browser: file_browser::BrowserView::default(),
            left_tab: left_panel::LeftTab::default(),
            samples: sample_browser::SampleBrowser::default(),
            plugins: left_panel::PluginList::default(),
            open_paths: Vec::new(),
            update_check: false,
            update_auto: true,
            bottom: Bottom::Mixer,
            status: None,
            tracks: tracks_area::TracksView::default(),
            mixer: mixer::MixerView {
                inputs: audio_rec::InputChoice::load(),
                ..mixer::MixerView::default()
            },
            group_watch: groups::GroupWatch::default(),
            inspector: inspector::InspectorView::default(),
            piano: piano_roll::PianoView::default(),
            steps: step_editor::StepView::default(),
            show_midi_fx: false,
            transform: transform_ui::TransformView::default(),
            score_tab: false,
            score: score::ScoreView::default(),
            screenset_on: None,
            score_heard: None,
            flex: flex_edit::FlexState::default(),
            file_edit: audio_file_editor::FileEditor::default(),
            groove: groove::GrooveState::default(),
            groove_make: groove_template::MakeState::default(),
            midi_cmd: None,
            piano_focus: false,
            audition: Vec::new(),
            sample_drop: None,
            bounce: bounce::BounceState::default(),
            rack: channel_rack::RackView::default(),
            settings: settings::SettingsView::default(),
            dialogs: quick_dialogs::QuickDialogs::default(),
            smart: smart_controls::SmartControls::default(),
            midi_learn: midi_learn::MidiLearn::default(),
            stems: stem_splitter::StemSplitter::default(),
            spatial: spatial_window::SpatialView::default(),
            gain_staging: gain_staging::GainStagingView::default(),
            organizer: session_organizer::OrganizerView::default(),
            performance: performance::PerformanceView::default(),
            layout: dock::Layout::load(),
            opening_plugins: None,
            opening_since: None,
            strip_silence: strip_silence::StripSilence::default(),
            keys: key_commands::KeyMap::default(),
            undo_labels: Vec::new(),
            redo_labels: Vec::new(),
            mastering: mastering::MasteringView::default(),
            tuner: tuner::Tuner::default(),
            typing: typing::Typing::default(),
            typed: Vec::new(),
            file_cmd: None,
            file_task: None,
            window_cmd: None,
            launching: false,
            ask_save: None,
            offer_new_extension: false,
            keep_old_extension: None,
            ask_revert: false,
            recent_pick: None,
            recent: recent_files::Recent::default(),
            cpu_load: 0.0,
            disk_load: 0.0,
            live_note_at: None,
            import_log: None,
            autosave_minutes: settings::autosave_minutes(),
            backups_kept: settings::backups_kept(),
            history_open: false,
            damaged: None,
            light_menus: false,
            chooser: None,
            new_details: None,
            chooser_devices: start_screen::FoundDevices::default(),
            thumbnail_wanted: None,
            thumbnail_settle: 0,
            sampler_window: None,
            sampler_view: sampler_window::SamplerView::default(),
            ui_scale: 1.0,
            undo_cmd: None,
            can_undo: false,
            can_redo: false,
        }
    }

    /// After a project is replaced: drop editor state (selections, open regions) that indexes
    /// into the old project, keeping which panels are shown.
    fn reset_views(&mut self) {
        self.status = None;
        self.tracks = tracks_area::TracksView::default();
        self.mixer = mixer::MixerView::default();
        self.group_watch.reset();
        self.inspector = inspector::InspectorView::default();
        self.piano = piano_roll::PianoView::default();
        self.piano_focus = false;
        self.audition.clear();
    }
}

struct App {
    /// Crash recovery: the copy kept while there are unsaved changes.
    autosave: autosave::Autosave,
    project: Project,
    ui: UiState,
    sync: Option<engine_sync::EngineSync>,
    /// The `.ovrtone` file the project was opened from or last saved to.
    path: Option<std::path::PathBuf>,
    /// Content at the last open/save, to detect unsaved changes (see `file_commands`).
    saved: ovrtone_project::ProjectDoc,
    /// macOS: the menus in the system menu bar (elsewhere they are drawn in the window).
    native_menu: Option<native_menu::NativeMenu>,
    /// `--bench <file>`: measuring startup, memory and DSP load, then closing.
    bench: Option<bench::Probe>,
    /// `--layout-shots <folder>`: pictures of the layout at several window sizes, then closing.
    shots: Option<bench::LayoutShots>,
    /// Edit > Undo / Redo.
    history: undo::History,
    /// Frames since the history last looked for an edit.
    frames_since_record: u32,
    /// The MIDI keyboard (none while benchmarking).
    midi: Option<midi_in::MidiIn>,
    /// Sends the notes of External MIDI tracks to their ports (none while benchmarking).
    midi_out: Option<midi_out::MidiOut>,
    /// The Mackie Control surface, while one is chosen and connected.
    surface: Option<control_surface::Surface>,
    /// When the chosen surface was last looked for.
    surface_tried: Option<std::time::Instant>,
    /// Reads MIDI Time Code from the project's port (none while benchmarking).
    mtc_in: Option<mtc_in::MtcIn>,
    /// Makes the transport follow that time code.
    chase: mtc_in::Chase,
    /// Looks for a newer release and installs it (Windows).
    updater: updater::Updater,
}

impl App {
    /// File > Import > MIDI File… and File > Export > Selection as MIDI File…
    fn run_midi_cmd(&mut self, cmd: midi_file::MidiCmd) {
        let (p, s) = (&mut self.project, &mut self.ui);
        match cmd {
            midi_file::MidiCmd::Import => {
                let picked = rfd::FileDialog::new()
                    .add_filter(i18n::tr("MIDI File"), &["mid", "midi", "smf"])
                    .pick_file();
                if let Some(path) = picked {
                    s.status = Some(match midi_file::import_path(p, &path) {
                        Ok(n) => i18n::trf("imported {} tracks from {}", &[&n, &path.display()]),
                        Err(e) => i18n::trf("cannot import {}: {}", &[&path.display(), &e]),
                    });
                }
            }
            midi_file::MidiCmd::Export => {
                // The selected region, or all the MIDI of the project.
                let only = s.tracks.selected.filter(|(t, r)| {
                    p.tracks.get(*t).is_some_and(|tr| {
                        tr.kind != model::TrackKind::Audio && *r < tr.regions.len()
                    })
                });
                let picked = rfd::FileDialog::new()
                    .add_filter(i18n::tr("MIDI File"), &["mid"])
                    .set_file_name(format!("{}.mid", p.name))
                    .save_file();
                if let Some(path) = picked {
                    s.status = Some(match midi_file::export_path(p, only, &path) {
                        Ok(()) => i18n::trf("saved {}", &[&path.display()]),
                        Err(e) => i18n::trf("cannot write {}: {}", &[&path.display(), &e]),
                    });
                }
            }
        }
    }

    fn import(&mut self, path: &std::path::Path, ctx: &egui::Context) {
        if midi_file::is_midi_path(path) {
            self.ui.status = Some(match midi_file::import_path(&mut self.project, path) {
                Ok(n) => i18n::trf("imported {} tracks from {}", &[&n, &path.display()]),
                Err(e) => i18n::trf("cannot import {}: {}", &[&path.display(), &e]),
            });
            return;
        }
        if !import::is_audio_path(path) {
            self.ui.status = Some(i18n::trf("not an audio file: {}", &[&path.display()]));
            return;
        }
        match import::import_audio(&mut self.project, path, ctx) {
            Ok(()) => {
                // A loop follows the project's tempo.
                let t = self.project.tracks.len().saturating_sub(1);
                self.ui.flex.detect_tempo(&mut self.project, t, 0, ctx);
            }
            Err(e) => {
                self.ui.status = Some(i18n::trf("cannot import {}: {}", &[&path.display(), &e]));
            }
        }
    }
}

impl App {
    /// Records finished edits for Undo, and runs Undo / Redo. An edit is looked for when a
    /// gesture ends (button let go, key, menu) and every half second (menus in the macOS menu
    /// bar send no input events), never in the middle of a drag.
    fn track_history(&mut self, ctx: &egui::Context) {
        self.frames_since_record += 1;
        let (gesture_ended, dragging) = ctx.input(|i| {
            let ended = i.pointer.any_released()
                || i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key { pressed: true, .. }
                            | egui::Event::Text(_)
                            | egui::Event::Paste(_)
                    )
                });
            (ended, i.pointer.any_down())
        });
        let undo_cmd = self.ui.undo_cmd.take();
        // A recording is one edit: it is recorded when it ends, not on every frame.
        let recording = self.ui.rec.active() || self.ui.audio_rec.active();
        if !dragging
            && !recording
            && (gesture_ended || undo_cmd.is_some() || self.frames_since_record >= 30)
        {
            self.frames_since_record = 0;
            self.history.record(&self.project);
        }
        let changed = match undo_cmd {
            Some(undo::UndoCmd::Undo) => self.history.undo(&mut self.project),
            Some(undo::UndoCmd::Redo) => self.history.redo(&mut self.project),
            Some(undo::UndoCmd::Clear) => {
                self.history.clear(&self.project);
                false
            }
            None => false,
        };
        if changed {
            // Track and region numbers may point elsewhere now.
            self.ui.tracks.selected = None;
            self.ui.piano.selected.clear();
            ctx.request_repaint();
        }
        let (back, forward) = (
            std::mem::take(&mut self.ui.dialogs.undo_steps),
            std::mem::take(&mut self.ui.dialogs.redo_steps),
        );
        for _ in 0..back {
            self.history.undo(&mut self.project);
        }
        for _ in 0..forward {
            self.history.redo(&mut self.project);
        }
        if back + forward > 0 {
            self.ui.tracks.selected = None;
            self.ui.piano.selected.clear();
        }
        if self.ui.dialogs.undo_history {
            self.ui.undo_labels = self.history.undo_labels().to_vec();
            self.ui.redo_labels = self.history.redo_labels().to_vec();
        }
        self.ui.can_undo = self.history.can_undo();
        self.ui.can_redo = self.history.can_redo();
    }

    /// MIDI Time Code in: the transport follows the port the project chases, if any.
    fn follow_time_code(&mut self, ctx: &egui::Context) {
        let Some(input) = self.mtc_in.as_mut() else {
            return;
        };
        input.sync(self.project.mtc_in.as_deref());
        // The keyboard input does not take the port the time code comes from.
        if let Some(midi) = self.midi.as_mut() {
            midi.avoid(self.project.mtc_in.as_deref());
        }
        let mut events = Vec::new();
        input.poll(&mut events);
        self.chase
            .update(&mut self.project, &events, std::time::Instant::now());
        if self.chase.is_following() {
            // Look again soon enough to notice when the time code stops.
            ctx.request_repaint_after(mtc_in::STALL / 3);
        }
    }

    /// MIDI keyboard notes: play them on the selected instrument and, while recording, put
    /// them into the new region.
    fn play_and_record_midi(&mut self, ctx: &egui::Context) {
        // What was typed on the computer keyboard goes the way a MIDI keyboard's notes do.
        let mut input = std::mem::take(&mut self.ui.typed);
        let mut ages = vec![0.0; input.len()];
        let mut from: Vec<midi_in::NoteFrom> = vec![None; input.len()];
        let latency_us = self
            .sync
            .as_mut()
            .map_or(0, |sync| sync.engine.state().output_latency_us);
        let mut controls = Vec::new();
        if let Some(midi) = self.midi.as_mut() {
            midi.poll(&mut input, &mut ages, &mut from);
        }
        // Armed tracks that listen to a device or channel of their own each play and record what
        // they hear; the first goes the usual way below, the others here.
        let routed = midi_routing::split(&self.project, &input, &ages, &from);
        let mut first_track = None;
        let mut others = Vec::new();
        if let Some(mut tracks) = routed {
            let (t, notes, note_ages) = tracks.remove(0);
            first_track = Some(t);
            input = notes;
            ages = note_ages;
            let latency = f64::from(latency_us) / 1e6;
            let p = &mut self.project;
            let s = &mut self.ui;
            for (t, notes, note_ages) in tracks {
                midi_routing::audition(&mut s.audition, t, &notes);
                let rec = s.rec_more.entry(t).or_default();
                rec.only_track = Some(t);
                rec.timing(note_ages, latency);
                recorder::update(rec, p, &notes);
                others.push(t);
            }
        }
        // Tracks that stopped listening on their own finish what they were recording, then go.
        {
            let p = &mut self.project;
            let more = &mut self.ui.rec_more;
            for (t, rec) in more.iter_mut() {
                if !others.contains(t) {
                    recorder::update(rec, p, &[]);
                }
            }
            more.retain(|t, rec| others.contains(t) || rec.active());
        }
        if !input.is_empty() {
            self.ui.live_note_at = Some(std::time::Instant::now());
        }
        if let Some(midi) = self.midi.as_mut() {
            midi.poll_controls(&mut controls);
        }
        let p = &mut self.project;
        let s = &mut self.ui;
        s.rec.timing(ages, f64::from(latency_us) / 1e6);
        let said = midi_learn::apply(p, &mut s.midi_learn, &controls, &mut s.smart.moves);
        if said.is_some() {
            s.status = said;
        }
        let track = s
            .rec
            .target()
            .map(|(t, _)| t)
            .or(first_track)
            .unwrap_or(p.selected_track);
        let plays = p
            .tracks
            .get(track)
            .is_some_and(|t| t.kind != model::TrackKind::Audio);
        for note in &input {
            if !plays {
                break;
            }
            s.audition.push(match *note {
                recorder::LiveNote::On { pitch, vel } => piano_roll::Audition::On {
                    pitch,
                    vel,
                    track: track.min(usize::from(u16::MAX)) as u16,
                },
                recorder::LiveNote::Off { pitch } => piano_roll::Audition::Off { pitch },
                recorder::LiveNote::Sustain { val } => piano_roll::Audition::Control {
                    track: track.min(usize::from(u16::MAX)) as u16,
                    number: 64,
                    value: val,
                },
            });
        }
        // Step input in the Piano Roll: the notes go in at its cursor instead of being recorded.
        if s.piano.step_input && s.bottom == Bottom::PianoRoll && !s.rec.active() {
            if let Some(at) = s.tracks.selected {
                let played: Vec<(u8, u8)> = input
                    .iter()
                    .filter_map(|n| match *n {
                        recorder::LiveNote::On { pitch, vel } => Some((pitch, vel)),
                        _ => None,
                    })
                    .collect();
                piano_roll::step_input(p, &mut s.piano, at, &played);
                piano_roll::step_keys(p, &mut s.piano, at, ctx);
                if !played.is_empty() {
                    ctx.request_repaint();
                }
                // Placed, so not recorded too.
                input.retain(|n| !matches!(n, recorder::LiveNote::On { .. }));
            }
        }
        // Record on an audio track is the audio recorder's (see `record_audio`).
        // Both record when the selected track is an instrument and an audio track is armed.
        let on_midi = p.tracks.get(p.selected_track).is_some_and(|t| {
            matches!(
                t.kind,
                model::TrackKind::Instrument | model::TrackKind::ExternalMidi
            )
        });
        let midi = if s.rec.active() || on_midi || !audio_rec::takes_over(p) {
            recorder::update(&mut s.rec, p, &input)
        } else {
            recorder::Outcome::Nothing
        };
        match midi {
            recorder::Outcome::Started(t, r) => {
                s.tracks.selected = Some((t, r));
                s.piano.selected.clear();
            }
            recorder::Outcome::Finished(kept) => {
                s.tracks.selected = kept;
                s.piano.selected.clear();
            }
            recorder::Outcome::NoTrack => {
                s.status = Some(i18n::tr("Record: there is no instrument track").into());
            }
            recorder::Outcome::Nothing => {}
        }
        if std::mem::take(&mut s.capture_cmd) {
            let on_audio = p
                .tracks
                .get(p.selected_track)
                .is_some_and(|t| t.kind == model::TrackKind::Audio);
            let made = if on_audio {
                match s.audio_rec.capture_played(p, ctx) {
                    Some(region) => Some(region),
                    None => recorder::capture(&mut s.rec, p),
                }
            } else {
                match recorder::capture(&mut s.rec, p) {
                    Some(region) => Some(region),
                    None => s.audio_rec.capture_played(p, ctx),
                }
            };
            match made {
                Some(region) => s.tracks.selected = Some(region),
                None => s.status = Some(i18n::tr("Capture Recording: nothing was played").into()),
            }
        }
        if s.rec.active() || !input.is_empty() || !s.rec_more.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Audio inputs: opens the input while a track wants it and records the armed tracks.
    fn record_audio(&mut self, ctx: &egui::Context) {
        let latency_us = self
            .sync
            .as_mut()
            .map_or(0, |sync| sync.engine.state().output_latency_us);
        let s = &mut self.ui;
        match s
            .audio_rec
            .update(&mut s.mixer.inputs, &mut self.project, latency_us, ctx)
        {
            audio_rec::Outcome::Finished(kept, warning) => {
                if kept.is_some() {
                    s.tracks.selected = kept;
                }
                if warning.is_some() {
                    s.status = warning;
                }
            }
            audio_rec::Outcome::Failed(message) => s.status = Some(message),
            audio_rec::Outcome::Nothing => {}
        }
        if let (Some(feed), Some(sync)) = (s.audio_rec.take_feed(), self.sync.as_mut()) {
            sync.engine
                .send(ovrtone_engine::Command::SetInput(Some(feed)));
        }
        if s.audio_rec.active() {
            ctx.request_repaint();
        } else if s.audio_rec.listening() {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }
    }

    /// The window while the Project Chooser is up: the menu bar and the chooser, with the
    /// File key commands and prompts still working.
    fn chooser_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(menu) = self.native_menu.as_mut() {
            menu.update(ctx, &mut self.project, &mut self.ui);
        }
        if let Some(cmd) = file_commands::pressed(ctx) {
            self.ui.file_cmd = Some(cmd);
        }
        self.ui.stems.hold_playback(&mut self.project);
        if let Some(sync) = self.sync.as_mut() {
            sync.sync(&mut self.project);
        }
        // Audio files dropped on the chooser go into the open project.
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if !dropped.is_empty() {
            self.ui.chooser = None;
            for path in dropped {
                self.import(&path, ctx);
            }
        }
        top_bar(ui, &mut self.project, &mut self.ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| start_screen::show(ui, &mut self.ui));
        if let Some(cmd) = self.ui.window_cmd.take() {
            menus::run_window_cmd(ctx, cmd);
        }
        if let Some(task) = self.ui.file_task.take() {
            self.run_file_task(task, ctx);
        }
        if let Some(cmd) = self.ui.file_cmd.take() {
            self.run_file_cmd(cmd, ctx);
        }
        self.show_new_extension_offer(ctx);
        self.show_save_prompt(ctx);
        window_chrome::resize_edges(ctx);
    }
}

impl App {
    /// Gives the Mastering window a copy of the output while it is open.
    /// Gives the effect window in front a copy of its effect's input and output (the analyzer and
    /// the moving levels), and stops it when that window closes or another comes to the front.
    fn connect_fx_listen(&mut self) {
        let want = self.ui.mixer.fx_window.filter(|(t, s)| {
            self.project
                .strip(*t)
                .is_some_and(|tr| *s < tr.inserts.len())
        });
        let have = self.ui.mixer.listen.as_ref().map(|l| l.key);
        // A tap that hears nothing was lost (its strip had no effects yet): ask again.
        let lost = self.ui.mixer.listen.as_ref().is_some_and(|l| l.lost());
        if want == have && !lost {
            return;
        }
        let Some(sync) = self.sync.as_mut() else {
            return;
        };
        if let Some((t, _)) = have {
            let stop = ovrtone_engine::Command::SetFxTap {
                track: t as u16,
                tap: None,
            };
            if !sync.engine.send(stop) {
                return;
            }
            self.ui.mixer.listen = None;
        }
        if let Some((t, s)) = want {
            let rate = sync.engine.sample_rate();
            let (tx, rx) = rtrb::RingBuffer::new(fx_ui::Listen::capacity(rate));
            let start = ovrtone_engine::Command::SetFxTap {
                track: t as u16,
                tap: Some(ovrtone_engine::FxTap {
                    slot: s,
                    out: Box::new(tx),
                }),
            };
            if sync.engine.send(start) {
                self.ui.mixer.listen = Some(fx_ui::Listen::new((t, s), rx, rate));
            }
        }
    }

    fn connect_mastering(&mut self) {
        // Always on, for the loudness on Stereo Out (the Mastering window and the Spatial Renderer
        // show it too). The measuring is on this thread; the engine only hands the samples over.
        let wanted = true;
        // Integrated loudness starts again each time playback starts, as in Logic.
        let playing = self.project.playing;
        if playing && !self.ui.mastering_was_playing {
            if let Some(sync) = self.sync.as_ref() {
                self.ui.mastering.reset(sync.engine.sample_rate());
            }
        }
        self.ui.mastering_was_playing = playing;
        // Read every frame, the mixer shown or not, so integrated loudness misses nothing.
        let _ = self.ui.mastering.readings();
        if wanted == self.ui.mastering.tap.is_some() {
            return;
        }
        let Some(sync) = self.sync.as_mut() else {
            return;
        };
        if wanted {
            let rate = sync.engine.sample_rate();
            let (tx, rx) = rtrb::RingBuffer::new(mastering::MasteringView::tap_capacity(rate));
            if sync
                .engine
                .send(ovrtone_engine::Command::SetMasterTap(Some(Box::new(tx))))
            {
                self.ui.mastering.tap = Some(rx);
                self.ui.mastering.reset(rate);
            }
        } else if sync
            .engine
            .send(ovrtone_engine::Command::SetMasterTap(None))
        {
            self.ui.mastering.tap = None;
        }
    }

    /// Changes the audio driver, device or buffer size: a new engine, which plays the project from
    /// where the old one was. If the choice will not start, the usual device plays and it says so.
    /// The control surface: opened when one is chosen (looked for again every few seconds while it
    /// is not there), its port kept out of note input, and run once a frame.
    fn run_surface(&mut self, levels: &[f32]) {
        let wanted = self.ui.surface_port.clone();
        if self
            .surface
            .as_ref()
            .is_some_and(|s| s.hui != self.ui.surface_hui)
        {
            self.surface = None;
            self.surface_tried = None;
        }
        if self.surface.as_ref().map(|s| &s.port) != wanted.as_ref() {
            self.surface = None;
        }
        if let Some(midi) = self.midi.as_mut() {
            midi.avoid_surface(wanted.as_deref());
        }
        let Some(port) = wanted else {
            return;
        };
        if self.surface.is_none() {
            let due = self
                .surface_tried
                .is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(3));
            if !due {
                return;
            }
            self.surface_tried = Some(std::time::Instant::now());
            match control_surface::Surface::open(&port) {
                Ok(surface) => {
                    self.surface = Some(surface);
                    self.ui.status = Some(format!("Control surface: {port}"));
                }
                Err(e) => tracing::info!("control surface {port}: {e}"),
            }
        }
        if let Some(surface) = self.surface.as_mut() {
            if surface.update(&mut self.project, levels) {
                self.project.song_rev += 1;
            }
        }
    }

    /// The audio device went away (unplugged, or the computer slept): Ovrtone stops cleanly, says
    /// so, and starts again on the same device when it is back, looking every two seconds.
    fn watch_audio_device(&mut self, ctx: &egui::Context) {
        if self.sync.as_ref().is_some_and(|s| s.engine.device_lost()) {
            if let Some(sync) = self.sync.as_mut() {
                sync.collect_plugin_states(&mut self.project);
            }
            self.project.playing = false;
            self.project.recording = false;
            self.midi_out = None;
            self.ui.mastering.tap = None;
            self.sync = None;
            self.ui.audio_lost = Some(std::time::Instant::now());
            self.ui.status = Some(
                "The audio device went away. Ovrtone has stopped, and starts again when it is back."
                    .to_string(),
            );
        }
        let Some(since) = self.ui.audio_lost else {
            return;
        };
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        if since.elapsed() < std::time::Duration::from_secs(2) {
            return;
        }
        self.ui.audio_lost = Some(std::time::Instant::now());
        let setup = self.ui.settings.setup.clone();
        // Only the device that went away: not the default in its place.
        if let Ok(engine) = ovrtone_engine::Engine::start_chosen(
            &setup.system,
            &setup.device,
            setup.buffer,
            setup.rate,
        ) {
            self.ui.audio_lost = None;
            self.adopt_engine(engine);
            self.ui.status = Some("The audio device is back.".to_string());
        }
    }

    /// Runs the project on `engine`.
    fn adopt_engine(&mut self, engine: ovrtone_engine::Engine) {
        let mut sync = engine_sync::EngineSync::new(engine, &self.project);
        self.midi_out = sync
            .take_midi_out()
            .zip(sync.take_mtc_out())
            .map(|(notes, mtc)| midi_out::MidiOut::start(notes, mtc));
        self.sync = Some(sync);
    }

    fn restart_audio(&mut self, setup: &settings::AudioSetup) {
        self.ui.audio_lost = None;
        if let Some(sync) = self.sync.as_mut() {
            sync.collect_plugin_states(&mut self.project);
        }
        self.project.playing = false;
        self.midi_out = None;
        self.ui.mastering.tap = None;
        // The old one lets go of the device before the new one asks for it.
        self.sync = None;
        let chosen = ovrtone_engine::Engine::start_chosen(
            &setup.system,
            &setup.device,
            setup.buffer,
            setup.rate,
        );
        let (started, failed) = match chosen {
            Ok(engine) => (Ok(engine), None),
            Err(e) => (ovrtone_engine::Engine::start(), Some(e)),
        };
        match started {
            Ok(engine) => {
                let mut sync = engine_sync::EngineSync::new(engine, &self.project);
                self.midi_out = sync
                    .take_midi_out()
                    .zip(sync.take_mtc_out())
                    .map(|(notes, mtc)| midi_out::MidiOut::start(notes, mtc));
                self.ui.status = Some(match failed {
                    Some(e) => format!("That audio choice did not start ({e}): using the default"),
                    None => format!("Audio: {}", sync.engine.device_name()),
                });
                self.sync = Some(sync);
            }
            Err(e) => {
                self.ui.status = Some(i18n::trf("No sound: {}. Check your audio device.", &[&e]));
            }
        }
    }
}

impl App {
    /// Takes the picture of the project that the chooser shows, a few frames after it was opened or
    /// saved so that it shows the project and not what came before.
    fn take_thumbnail(&mut self, ctx: &egui::Context) {
        if self.ui.thumbnail_settle > 0 {
            self.ui.thumbnail_settle -= 1;
            ctx.request_repaint();
        }
        if self.ui.chooser.is_some() {
            return;
        }
        let Some((path, frames)) = self.ui.thumbnail_wanted.take() else {
            return;
        };
        if frames > 0 {
            self.ui.thumbnail_wanted = Some((path, frames - 1));
            ctx.request_repaint();
            return;
        }
        project_thumbs::request(ctx, path);
        self.ui.thumbnail_settle = 10;
    }

    /// Holds the launch until the plugin scan is done, showing it on the launch screen: the main
    /// window opens with every plugin known and no scan going on. Later scans are only those the
    /// user asks for. Only Windows has the screen to show this on.
    fn wait_for_launch_scan(&mut self) {
        if !cfg!(windows) {
            return;
        }
        loop {
            let scan = &mut self.ui.plugins.clap;
            scan.drain();
            if scan.progress.is_none() {
                break;
            }
            show_scan_on_launch_screen(scan);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        launch_step("Opening the window", 0.97);
    }

    /// Holds the launch until the plugins of the project asked for are loaded, or have failed, showing
    /// it on the launch screen: the window then opens ready, with nothing left to do behind it.
    fn load_plugins_on_launch_screen(&mut self) {
        if !cfg!(windows) {
            return;
        }
        let Some(sync) = self.sync.as_mut() else {
            return;
        };
        let total = sync.plugins_left(&mut self.project).0;
        // A plugin's licence or activation window comes up in front of the launch screen.
        if total > 0 {
            ovrtone_host::splash::let_others_over();
        }
        let (mut last, mut since) = (total, std::time::Instant::now());
        loop {
            let (left, next) = sync.plugins_left(&mut self.project);
            if left == 0 {
                break;
            }
            if left != last {
                (last, since) = (left, std::time::Instant::now());
            }
            // The audio thread takes them. If it stops taking them, what is left loads in the window.
            if since.elapsed() > std::time::Duration::from_secs(30) {
                break;
            }
            let done = total.saturating_sub(left);
            let name = next.unwrap_or_default();
            let part = done as f32 / total.max(1) as f32;
            launch_step(
                &format!("Loading plugins: {} of {total}, {name}", done + 1),
                0.95 + 0.04 * part,
            );
            sync.sync(&mut self.project);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// While a project's plugins load one by one, the window says so and takes no clicks, as they
    /// would go unanswered while each loads.
    fn plugin_load_screen(&mut self, ctx: &egui::Context) {
        let Some(sync) = self.sync.as_ref() else {
            return;
        };
        let (left, next) = sync.plugins_left(&mut self.project);
        if left == 0 {
            self.ui.opening_plugins = None;
            self.ui.opening_since = None;
            return;
        }
        let Some(most) = self.ui.opening_plugins.as_mut() else {
            self.ui.opening_since = None;
            return;
        };
        // Plugins in their own processes go on loading behind the window: after a while the user
        // can carry on while they do.
        let since = *self
            .ui
            .opening_since
            .get_or_insert_with(std::time::Instant::now);
        let can_leave = since.elapsed() >= std::time::Duration::from_secs(10);
        let mut leave = false;
        *most = (*most).max(left);
        let total = *most;
        ctx.request_repaint();
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("plugin_load_screen"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (r, resp) =
                    ui.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
                leave = can_leave && resp.clicked();
                let g = ui.painter();
                g.rect_filled(r, 0.0, egui::Color32::from_black_alpha(170));
                let text = theme::text();
                let c = r.center();
                let done = total - left;
                g.text(
                    c + egui::vec2(0.0, -16.0),
                    egui::Align2::CENTER_CENTER,
                    format!("Loading plugins: {} of {total}", done + 1),
                    egui::FontId::proportional(15.0),
                    text,
                );
                g.text(
                    c + egui::vec2(0.0, 22.0),
                    egui::Align2::CENTER_CENTER,
                    next.unwrap_or_default(),
                    egui::FontId::proportional(12.5),
                    theme::text_dim(),
                );
                let bar =
                    egui::Rect::from_center_size(c + egui::vec2(0.0, 3.0), egui::vec2(320.0, 4.0));
                g.rect_filled(bar, 2.0, theme::gray(0x40));
                let mut lit = bar;
                lit.set_width(bar.width() * done as f32 / total.max(1) as f32);
                g.rect_filled(lit, 2.0, theme::accent());
                if can_leave {
                    g.text(
                        c + egui::vec2(0.0, 48.0),
                        egui::Align2::CENTER_CENTER,
                        "Click to work while the rest load",
                        egui::FontId::proportional(12.0),
                        theme::text_dim(),
                    );
                }
            });
        if leave {
            self.ui.opening_plugins = None;
            self.ui.opening_since = None;
        }
    }

    /// The launch screen goes once the scan is done and the main window has drawn.
    fn follow_launch_scan(&mut self, ctx: &egui::Context) {
        if !self.ui.launching {
            return;
        }
        let scan = &mut self.ui.plugins.clap;
        scan.drain();
        if scan.progress.is_some() {
            show_scan_on_launch_screen(scan);
            return;
        }
        self.ui.launching = false;
        ovrtone_host::splash::close();
        // The window opened under the launch screen and may not have the keyboard: take it, or the
        // first keys go to whatever had it before (Windows answers those with its error sound).
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
}

fn show_scan_on_launch_screen(scan: &plugin_scan::PluginScan) {
    let Some((done, total)) = scan.progress else {
        return;
    };
    let name = scan.reading.lock().map(|n| n.clone()).unwrap_or_default();
    let words = if total == 0 {
        "Looking for plugins".to_string()
    } else if name.is_empty() {
        format!("Checking plugins: {done} of {total}")
    } else {
        // Only what is new is read. The rest of the program waits for it this once.
        format!("Scanning new plugins, this happens once: {done} of {total}, {name}")
    };
    let part = if total == 0 {
        0.0
    } else {
        done as f32 / total as f32
    };
    launch_step(&words, 0.3 + 0.65 * part);
}

impl eframe::App for App {
    /// A clean exit leaves nothing to recover.
    fn on_exit(&mut self) {
        self.autosave.forget();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.follow_launch_scan(ui.ctx());
        self.take_thumbnail(ui.ctx());
        let ctx = ui.ctx().clone();
        // Before anything else looks at the keys: while it is on, the note keys are its own.
        let mut typed = std::mem::take(&mut self.ui.typed);
        self.ui.typing.poll(&ctx, &mut typed);
        self.ui.typed = typed;
        mixer::refresh_installed(&ctx, &mut self.ui);
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F6)) {
            self.ui.rack.open = !self.ui.rack.open;
        }
        // F7 as in FL Studio.
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F7)) {
            self.ui.bottom = if self.ui.bottom == Bottom::PianoRoll {
                Bottom::Mixer
            } else {
                Bottom::PianoRoll
            };
        }
        let _frame_time = bench::frame_timer();
        let dt = ctx.input(|i| i.stable_dt) as f64;
        ui_scale::apply(&ctx, self.ui.ui_scale);
        tracks_area::poll_normalize(&ctx, &mut self.project, &mut self.ui);
        smart_tempo::poll(&ctx, &mut self.project, &mut self.ui);
        flex_edit::poll(&ctx, &mut self.project, &mut self.ui);
        audio_file_editor::poll(&ctx, &mut self.project, &mut self.ui);
        audio_file_editor::show(&ctx, &mut self.project, &mut self.ui);
        groove::poll(&ctx, &mut self.project, &mut self.ui);
        groove_template::poll(&ctx, &mut self.project, &mut self.ui);
        menus::expire_status(&ctx, &mut self.ui);
        beat_store::poll(&mut self.ui);
        beat_store::dialog(&ctx, &self.project, &mut self.ui);
        // Tuner: keep its tap in sync with its open state, then run its detection.
        let tuner_open = self.ui.tuner.open;
        if let Some(sync) = self.sync.as_mut() {
            tuner::set_tap(&mut sync.engine, &mut self.ui.tuner, tuner_open);
        }
        tuner::tick(&mut self.ui.tuner);
        tuner::window(&ctx, &mut self.ui.tuner);
        if !self.ui.launching {
            coming_from::on_launch(&mut self.ui);
        }
        coming_from::window(&ctx, &mut self.ui);
        clip_launcher::window(&ctx, &mut self.project, &mut self.ui);
        feedback::window(&ctx, &mut self.ui);
        // MIDI from Melodyne: the plugin is asked for the notes it found.
        if let Some(t) = self.ui.ara_midi.take() {
            let result = match self.sync.as_mut() {
                Some(sync) => sync.ara_notes(t),
                None => Err("No audio engine".to_string()),
            };
            self.ui.status = Some(match result {
                Ok(notes) => {
                    let n = flex_edit::midi_track_from_song(&mut self.project, t, &notes);
                    format!("MIDI from Melodyne: {n} notes on a new track")
                }
                Err(e) => e,
            });
        }
        if let Some(probe) = self.bench.as_mut() {
            if probe.frame(&ctx) {
                self.bench = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if let Some(shots) = self.shots.as_mut() {
            if shots.frame(&ctx) {
                self.shots = None;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        self.autosave_tick();
        self.show_recovery(&ctx);
        self.show_history(&ctx);
        if self.ui.chooser.is_some() {
            self.chooser_ui(ui, &ctx);
            return;
        }
        // Space plays and stops wherever the pointer or the last click is, as in every DAW. It is
        // taken here, before any widget sees it: a button clicked last (a plugin slot, M, S) has the
        // keyboard focus, and would otherwise be "pressed" by Space instead.
        if !ctx.egui_wants_keyboard_input()
            && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        {
            self.project.playing = !self.project.playing;
            ctx.memory_mut(|m| {
                if let Some(id) = m.focused() {
                    m.surrender_focus(id);
                }
            });
        }
        if let Some(menu) = self.native_menu.as_mut() {
            menu.update(&ctx, &mut self.project, &mut self.ui);
        }
        if let Some(cmd) = file_commands::pressed(&ctx) {
            self.ui.file_cmd = Some(cmd);
        }
        // Before the key commands: while it is open, its letters pick tools.
        tracks_area::tool_menu(&ctx, &mut self.ui);
        menus::handle_keys(&ctx, &mut self.project, &mut self.ui);
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        // One sample dragged in from File Explorer asks what it becomes, as a sample from the
        // Browser does; several at once are imported as audio clips.
        if let [path] = dropped.as_slice() {
            if import::is_audio_path(path) {
                let at = ctx
                    .input(|i| i.pointer.hover_pos())
                    .unwrap_or_else(|| ctx.content_rect().center());
                let bar = self
                    .project
                    .beats_to_bar(self.project.position_beats)
                    .floor()
                    + 1.0;
                self.ui.sample_drop = Some(sample_place::SampleDrop {
                    path: path.clone(),
                    at,
                    bar,
                    track: Some(self.project.selected_track),
                    ask: false,
                });
            } else {
                self.import(path, &ctx);
            }
        } else {
            for path in dropped {
                self.import(&path, &ctx);
            }
        }
        if let Some(out) = self.midi_out.as_mut() {
            out.sync(&self.project);
        }
        self.follow_time_code(&ctx);
        // Pattern and Drummer regions follow their length and the bars under them.
        self.project.refresh_generated();
        takes::record_commands(&mut self.project, &mut self.ui);
        self.ui.stems.hold_playback(&mut self.project);
        self.plugin_load_screen(&ctx);
        if let Some(sync) = self.sync.as_mut() {
            sync.sync(&mut self.project);
            if self.ui.spatial.open || self.ui.spatial.panner_open {
                self.ui.spatial.device_channels = sync.output_channels();
                self.ui.spatial.hrtf_error.clone_from(&sync.hrtf_error);
                let peaks = sync.take_spatial_peaks();
                for (now, new) in self.ui.spatial.peaks.iter_mut().zip(peaks) {
                    *now = new.max(*now * 0.85);
                }
            }
            sync.set_pre_fader(self.ui.mixer.pre_fader);
            for (uid, param, value) in std::mem::take(&mut self.ui.smart.moves) {
                sync.smart_move(uid, param, value);
            }
            for (uid, param) in self.ui.smart.wanted.clone() {
                if let Some(value) = sync.param_value(uid, param) {
                    self.ui.smart.values.insert((uid, param), value);
                }
            }
            if let Some(cmd) = self.ui.mixer.strip_cmd.take() {
                use plugin_presets::StripCmd;
                sync.collect_plugin_states(&mut self.project);
                match cmd {
                    StripCmd::Copy { track } => {
                        if let Some(t) = self.project.tracks.get(track) {
                            self.ui.mixer.copied = Some(mixer::CopiedStrip::of(t));
                        }
                    }
                    StripCmd::Save { track, file } => {
                        if let Some(t) = self.project.tracks.get(track) {
                            let written = project_io::strip_to_text(t).and_then(|text| {
                                std::fs::write(&file, text).map_err(|e| e.to_string())
                            });
                            if let Err(e) = written {
                                self.ui.status = Some(e);
                            }
                        }
                    }
                }
            }
            if let Some(e) = self.ui.mixer.strip_error.take() {
                self.ui.status = Some(e);
            }
            if let Some(cmd) = self
                .ui
                .mixer
                .preset_cmd
                .take()
                .or_else(|| self.ui.browser.preset_cmd.take())
            {
                if let Err(e) = sync.run_preset(&cmd) {
                    self.ui.status = Some(e);
                }
            }
            self.ui.mixer.showing_b = sync.showing_b();
            let resets = std::mem::take(&mut self.ui.mixer.clip_resets);
            for reset in resets {
                // One the engine cannot take now is tried again next frame.
                if !sync.reset_clip(reset) {
                    self.ui.mixer.clip_resets.push(reset);
                }
            }
            if let Some((uid, name)) = self.ui.bus_plugin_editor.take() {
                sync.open_bus_plugin_editor(uid, &name, main_window_handle(frame));
            }
            if let Some((uid, name)) = self.ui.mixer.open_plugin.take() {
                // A sample is shown by a window of the program's own.
                let sample = self.project.all_hosted_mut().any(|i| {
                    i.uid == uid
                        && ovrtone_host::PluginFormat::of(&i.path)
                            == ovrtone_host::PluginFormat::Sampler
                });
                if sample {
                    self.ui.sampler_window = Some(uid);
                } else {
                    sync.open_plugin_editor(uid, &name, main_window_handle(frame));
                }
            }
            sampler_window::show(&ctx, &mut self.project, &mut self.ui, sync);
            if let Some(message) = sync.take_plugin_message() {
                self.ui.status = Some(message);
            }
            if let Some(after) = sync.plugins_want_repaint_after() {
                ctx.request_repaint_after(after);
            }
            automation::tick(&mut self.project);
            groups::propagate(&mut self.project, &mut self.ui.group_watch);
            if sync.needs_retry() {
                ctx.request_repaint();
            }
            if let Some((track, beat)) = self.ui.tracks.scrub.take() {
                let rate = sync.engine.sample_rate();
                if let Some(samples) = region_edit::scrub_grain(&self.project, track, beat, rate) {
                    sync.engine
                        .send(ovrtone_engine::Command::SetPreview(Some(Box::new(
                            ovrtone_engine::Preview { samples, gain: 0.8 },
                        ))));
                }
            }
            let load = sync.take_dsp_load();
            performance::sample(
                &mut self.ui.performance,
                sync,
                self.project.tracks.len(),
                load,
            );
            // Render ahead only while nothing is live: no recording, no armed or monitoring track,
            // and no notes played in for a few seconds.
            let live = self.project.recording
                || self.project.tracks.iter().any(|t| t.rec || t.monitor)
                || self
                    .ui
                    .live_note_at
                    .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(3));
            let ahead = if settings::render_ahead() && self.project.playing && !live {
                sync.engine.sample_rate() / 20
            } else {
                0
            };
            sync.engine.set_render_ahead(ahead);
            self.ui.cpu_load = load.max(self.ui.cpu_load - dt as f32 * 0.8);
            self.ui.disk_load = sync
                .engine
                .disk_load()
                .max(self.ui.disk_load - dt as f32 * 0.8);
            // Only when it happens, not on every frame from then on.
            let underruns = sync.underruns();
            if underruns > self.ui.underruns_told {
                self.ui.status = Some(i18n::trf(
                    "disk too slow: {} underruns",
                    &[&(underruns - self.ui.underruns_told)],
                ));
            }
            self.ui.underruns_told = underruns;
        } else if self.project.playing {
            performance::no_engine(&mut self.ui.performance);
            // No audio device: fall back to a UI-driven clock.
            let map = self.project.tempo_map();
            self.project.position_beats = map.advance(self.project.position_beats, dt);
        }
        if self.project.playing {
            ctx.request_repaint();
        }
        let master_peaks = self.sync.as_mut().map(|s| s.master_peaks());
        let mut meters = match self.sync.as_mut() {
            Some(sync) => mixer::Meters {
                tracks: sync.track_meters(self.project.tracks.len()),
                rms: sync.track_rms(self.project.tracks.len()),
                reduction: sync.gain_reduction(self.project.tracks.len()),
                master: sync.master_peaks(),
                clips: sync.track_clips(self.project.tracks.len()),
                master_clip: sync.master_clip(),
            },
            None => mixer::Meters::default(),
        };
        self.ui
            .audio_rec
            .show_levels(&self.project, &mut meters.tracks);
        let p = &mut self.project;
        let s = &mut self.ui;

        top_bar(ui, p, s);
        egui::Panel::top("control")
            .exact_size(58.0)
            .frame(egui::Frame::new().fill(theme::toolbar()))
            .show(ui, |ui| transport::control_bar(ui, p, s, master_peaks));
        let mut cards: Vec<egui::Rect> = Vec::new();
        // Each panel where the layout puts it (Window > Panel Layout).
        let mut layout = std::mem::take(&mut s.layout);
        if s.show_library {
            let size = dock::Size {
                default: 420.0,
                range: 330.0..=720.0,
                exact: None,
            };
            let shown = dock::show(
                ui,
                &mut layout,
                dock::Pane::Library,
                size,
                egui::Frame::new(),
                |ui| left_panel::show(ui, p, s),
            );
            cards.extend(shown);
        }
        if s.show_inspector {
            let size = dock::Size {
                default: 244.0,
                range: 244.0..=244.0,
                exact: Some(244.0),
            };
            let shown = dock::show(
                ui,
                &mut layout,
                dock::Pane::Inspector,
                size,
                egui::Frame::new(),
                |ui| inspector::show(ui, p, s, &meters),
            );
            cards.extend(shown);
        }
        if s.show_toolbar {
            egui::Panel::top("toolbar")
                .exact_size(side_panes::TOOLBAR_HEIGHT)
                .show(ui, |ui| side_panes::toolbar(ui, p, s));
        }
        if s.browser.open.is_some() {
            let size = dock::Size {
                default: file_browser::WIDTH,
                range: 220.0..=560.0,
                exact: None,
            };
            dock::show(
                ui,
                &mut layout,
                dock::Pane::Files,
                size,
                egui::Frame::side_top_panel(ui.style()),
                |ui| file_browser::show(ui, p, s),
            );
        }
        if s.show_notes {
            let size = dock::Size {
                default: side_panes::NOTES_WIDTH,
                range: 180.0..=560.0,
                exact: None,
            };
            dock::show(
                ui,
                &mut layout,
                dock::Pane::Notes,
                size,
                egui::Frame::side_top_panel(ui.style()),
                |ui| side_panes::note_pad(ui, p),
            );
        }
        if s.show_lists {
            let size = dock::Size {
                default: event_list::WIDTH,
                range: event_list::WIDTH..=640.0,
                exact: None,
            };
            dock::show(
                ui,
                &mut layout,
                dock::Pane::Lists,
                size,
                egui::Frame::new(),
                |ui| event_list::show(ui, p, s),
            );
        }
        if s.show_mixer || s.bottom == Bottom::PianoRoll {
            // More than half of the space under the control bar (620 points at most), so the Tracks
            // area keeps room on short screens. Pull it taller for longer faders; it stays inside
            // the window when the window shrinks.
            let room = ui.available_height();
            let usual = (room * 0.55).clamp(160.0, 620.0);
            let size = dock::Size {
                default: usual,
                range: 160.0..=(room * 0.85).max(160.0),
                exact: None,
            };
            let shown = dock::show(
                ui,
                &mut layout,
                dock::Pane::Bottom,
                size,
                egui::Frame::new(),
                |ui| match s.bottom {
                    Bottom::Mixer => mixer::show(ui, p, s, &meters),
                    Bottom::PianoRoll => piano_roll::show(ui, p, s),
                },
            );
            cards.extend(shown);
        }
        dock::layout_window(&ctx, &mut layout);
        s.layout = layout;
        let shown = egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ui, |ui| tracks_area::show(ui, p, s, &meters));
        cards.push(shown.response.rect);
        let edges = ui.painter().clone();
        for card in cards {
            theme::panel_edge(&edges, card);
        }
        sample_place::menu(ui, p, s);
        bounce::show(&ctx, p, s);
        channel_rack::show(&ctx, p, s);
        settings::show(&ctx, s);
        if let Some(label) = s.tracks.pending_menu.take() {
            menus::apply(label, p, s);
        }
        if let Some(index) = s.tracks.groove_quantize.take() {
            let n = groove_template::quantize_selected(p, s, index);
            s.status = Some(format!("Quantized {n} regions to the groove"));
        }
        if std::mem::take(&mut s.mixer.open_midi_fx) {
            s.show_midi_fx = true;
        }
        if let Some(track) = s.mixer.rename.take() {
            if let Some(name) = p.tracks.get(track).map(|t| t.name.clone()) {
                s.dialogs.open_rename(track, &name);
            }
        }
        if let Some((track, as_send)) = s.mixer.new_aux.take() {
            if p.new_aux_for(track, as_send).is_none() {
                s.status = Some("Cannot make a new Aux for this strip".to_string());
            }
        }
        quick_dialogs::show(&ctx, p, s);
        smart_controls::show(&ctx, p, s);
        midi_learn::show(&ctx, p, s);
        stem_splitter::show(&ctx, p, s);
        strip_silence::show(&ctx, p, s);
        mastering::show(&ctx, s);
        if std::mem::take(&mut s.mixer.open_renderer) {
            s.spatial.open = true;
        }
        if std::mem::take(&mut s.mixer.open_spatial) {
            s.spatial.panner_open = true;
        }
        spatial_window::show(&ctx, p, s);
        if let Some(m) = s.tracks.stretch_status.take() {
            s.status = Some(m);
        }
        if std::mem::take(&mut s.tracks.consolidate) {
            bounce::bounce_selection(p, s, true);
            s.status = Some("Consolidating the selected regions into one".to_string());
        }
        if let Some(task) = s.mixer.file_task.take() {
            s.file_task = Some(task);
        }
        if std::mem::take(&mut s.mixer.open_gain_staging) {
            s.gain_staging.open = true;
        }
        gain_staging::show(&ctx, p, s);
        session_organizer::show(&ctx, p, s);
        performance::show(&ctx, p, s);
        import_log_window(&ctx, s);
        fx_ui::show(&ctx, p, s, &meters);
        let levels: Vec<f32> = meters.tracks.iter().map(|(l, r)| l.max(*r)).collect();
        automation::show(&ctx, p, s);

        self.run_surface(&levels);
        side_panes::quick_help(&ctx, &mut self.ui);
        for path in std::mem::take(&mut self.ui.open_paths) {
            self.import(&path, &ctx);
        }
        if let Some(setup) = self.ui.settings.apply.take() {
            self.restart_audio(&setup);
        }
        self.watch_audio_device(&ctx);
        if let Some(driver) = self.ui.settings.panel_wanted.take() {
            // The driver has to be free before its panel can have it.
            if let Some(sync) = self.sync.as_mut() {
                sync.collect_plugin_states(&mut self.project);
            }
            self.project.playing = false;
            self.midi_out = None;
            self.ui.mastering.tap = None;
            self.sync = None;
            match ovrtone_engine::AsioPanel::open(&driver, main_window_handle(frame)) {
                Ok(panel) => self.ui.settings.panel = Some(panel),
                Err(e) => {
                    let setup = self.ui.settings.setup.clone();
                    self.restart_audio(&setup);
                    self.ui.status = Some(e);
                }
            }
        }
        if std::mem::take(&mut self.ui.settings.panel_done) {
            self.ui.settings.panel = None;
            let setup = self.ui.settings.setup.clone();
            self.restart_audio(&setup);
        }
        self.connect_mastering();
        self.connect_fx_listen();
        if self.ui.settings.open {
            self.ui.settings.current = self.sync.as_ref().map_or("nothing".to_string(), |s| {
                format!(
                    "{} at {} Hz",
                    s.engine.device_name(),
                    s.engine.sample_rate()
                )
            });
        }
        // A render starts from the settings the plugins have now.
        if !self.ui.bounce.queue.is_empty() && self.ui.bounce.job.is_none() {
            if let Some(sync) = self.sync.as_mut() {
                sync.collect_plugin_states(&mut self.project);
            }
        }
        let rate = self.sync.as_ref().map_or(0, |s| s.engine.sample_rate());
        if let Some(message) = self.ui.bounce.poll(&mut self.project, rate, &ctx) {
            self.ui.status = Some(message);
        }
        // The Browser's preview goes to the engine (it has no engine without an audio device).
        let engine = self.sync.as_mut().map(|sync| &mut sync.engine);
        self.ui.samples.pump(engine, &ctx);
        self.record_audio(&ctx);
        self.project.follow_rolls();
        self.play_and_record_midi(&ctx);
        if std::mem::take(&mut self.ui.update_check) {
            self.updater.start_check(true);
        }
        let unsaved = || file_commands::content(&self.project) != self.saved;
        if self.updater.show(&ctx, &unsaved) == updater::Action::Quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Forward live notes (piano keys, note auditioning) to the engine.
        // Notes on tracks past the engine's limit are silent, like the rest of those tracks.
        // A rejected event stays queued (with everything after it, to keep On/Off order)
        // and is retried next frame, so a Note Off is never lost.
        if let Some(sync) = self.sync.as_mut() {
            let mut sent = 0;
            for ev in &self.ui.audition {
                let cmd = match *ev {
                    piano_roll::Audition::On { track, .. }
                        if usize::from(track) >= ovrtone_engine::MAX_TRACKS =>
                    {
                        None
                    }
                    piano_roll::Audition::On { pitch, vel, track } => {
                        Some(ovrtone_engine::Command::NoteOn { pitch, vel, track })
                    }
                    piano_roll::Audition::Off { pitch } => {
                        Some(ovrtone_engine::Command::NoteOff { pitch })
                    }
                    piano_roll::Audition::Control {
                        track,
                        number,
                        value,
                    } => Some(ovrtone_engine::Command::LiveControl {
                        track,
                        number,
                        value,
                    }),
                };
                if cmd.is_some_and(|cmd| !sync.engine.send(cmd)) {
                    break;
                }
                sent += 1;
            }
            self.ui.audition.drain(..sent);
            if !self.ui.audition.is_empty() {
                ctx.request_repaint();
            }
        } else {
            self.ui.audition.clear();
        }
        if let Some(cmd) = self.ui.window_cmd.take() {
            menus::run_window_cmd(&ctx, cmd);
        }
        if let Some(task) = self.ui.file_task.take() {
            self.run_file_task(task, &ctx);
        }
        if let Some(cmd) = self.ui.file_cmd.take() {
            self.run_file_cmd(cmd, &ctx);
        }
        if let Some(cmd) = self.ui.midi_cmd.take() {
            self.run_midi_cmd(cmd);
        }
        midi_fx_ui::show(&ctx, &mut self.project, &mut self.ui);
        transform_ui::show(&ctx, &mut self.project, &mut self.ui);
        smart_tempo::show(&ctx, &mut self.project, &mut self.ui);
        self.show_new_extension_offer(&ctx);
        self.show_save_prompt(&ctx);
        self.show_revert_prompt(&ctx);
        self.track_history(&ctx);
        window_chrome::resize_edges(&ctx);
    }
}

/// The in-window menu bar; on macOS, where the menus are in the system menu bar, only a line
/// for errors and warnings while there is one.
fn top_bar(ui: &mut egui::Ui, p: &mut Project, s: &mut UiState) {
    let frame = egui::Frame::side_top_panel(ui.style()).fill(menus::bar_fill(s.light_menus));
    if menus::IN_WINDOW {
        let bar = egui::Panel::top("menu")
            .frame(frame)
            .show(ui, |ui| menus::menu_bar(ui, p, s));
        if window_chrome::CUSTOM {
            let fg = menus::bar_text(s.light_menus);
            window_chrome::bar_controls(ui, bar.response.rect, bar.inner + 8.0, fg);
        }
    } else if menus::has_status(p, s) {
        egui::Panel::top("status")
            .frame(frame)
            .show(ui, |ui| ui.horizontal(|ui| menus::status_line(ui, p, s)));
    }
}

/// Log to stderr; `RUST_LOG` overrides the default (warnings, plus info from Ovrtone; the
/// Vulkan loader's harmless layer-manifest warnings from `wgpu_hal` are hidden).
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,wgpu_hal=error,ovrtone=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// The Ovrtone logo, 256×256 with transparency (the largest image in assets/ovrtone.ico).
const APP_ICON_PNG: &[u8] = include_bytes!("../assets/icon-256.png");

/// The Flex and Smart Tempo layout pictures (`OVRTONE_SHOT_SCENE=flex…`): a drum loop on an
/// audio track, made in the temporary folder, with Flex markers, a Flex Pitch MIDI track and
/// a Groove Track; `flexeditor` opens the Smart Tempo Editor on it, `flexmenu` the loop's
/// right-click menu and `flexgroove` a track's.
fn shot_flex_scene(app: &mut App, ctx: &egui::Context, kind: &str) {
    let dir = std::env::temp_dir().join("ovrtone-shots");
    let _ = std::fs::create_dir_all(&dir);
    let wav = dir.join("Drum Loop.wav");
    // Two bars of a 96 BPM beat: kick, snare and hats, a little late here and there.
    let rate = 44_100usize;
    let beat = 60.0 / 96.0;
    let frames = (beat * 8.0 * 2.0 * rate as f64) as usize;
    let mut v = vec![0f32; frames];
    let mut seed = 7u32;
    for k in 0..32 {
        let t = k as f64 * beat / 2.0 + if k % 4 == 3 { 0.025 } else { 0.0 };
        let at = (t * rate as f64) as usize;
        let (len, level, low) = match k % 4 {
            0 => (9000, 0.9, true),
            2 => (6000, 0.8, false),
            _ => (2500, 0.07, false),
        };
        for i in 0..len {
            if at + i >= frames {
                break;
            }
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
            let decay = (-(i as f32) / (len as f32 / 5.0)).exp();
            let body = if low {
                (i as f32 * 60.0 * std::f32::consts::TAU / rate as f32).sin()
            } else {
                noise
            };
            v[at + i] += body * level * decay;
        }
    }
    let stereo: Vec<f32> = v.iter().flat_map(|s| [*s, *s]).collect();
    if ovrtone_engine::write_wav(&wav, &stereo, rate as u32).is_err() {
        return;
    }
    let pr = &mut app.project;
    pr.position_beats = 0.0;
    if import::import_audio(pr, &wav, ctx).is_err() {
        return;
    }
    let t = pr.tracks.len() - 1;
    pr.tracks[t].regions[0].start_bar = 3.0;
    // Move the audio track near the top, where the pictures see it.
    let track = pr.tracks.remove(t);
    pr.tracks.insert(5, track);
    let t = 5;
    flex_edit::set_mode(pr, t, 0, ovrtone_engine::FlexMode::Rhythmic);
    if let Some(f) = pr.tracks[t].regions[0]
        .audio
        .as_mut()
        .and_then(|a| a.flex.as_mut())
    {
        f.markers = flex_edit::markers_from(
            (1..32).map(|k| (k as f64 * beat / 2.0, k as f64 * beat / 2.0 * 0.98)),
        );
    }
    groove::set_groove_track(pr, Some(t));
    pr.tracks[0].params.follow_groove = true;
    pr.selected_track = t;
    app.ui.tracks.selected = Some((t, 0));
    match kind {
        "editor" => smart_tempo::open_editor(&app.project, &mut app.ui, t, 0),
        "menu" => app.ui.tracks.shot_menu = Some((t, Some(0))),
        "groove" => app.ui.tracks.shot_menu = Some((0, None)),
        _ => {}
    }
}

/// The Mixer layout pictures (`OVRTONE_SHOT_SCENE=mix…`): the demo project playing, with two
/// Aux strips (a reverb and a delay), sends to them, effects on the strips, a compressor
/// working, and automation in its different modes. `mixfx` also opens the compressor's window,
/// `mixeq` the Channel EQ's and `mixauto` the Automation editor.
fn shot_mixer_scene(app: &mut App, kind: &str) {
    use model::{AutoMode, AutoTarget, Insert, SendSlot, TrackKind};
    use ovrtone_engine::FxKind;
    let p = &mut app.project;
    let aux = |p: &mut model::Project, name: &str, kind: FxKind, mix: usize| {
        let i = p.add_track(TrackKind::Aux);
        p.tracks[i].name = name.into();
        let mut fx = Insert::new(kind);
        fx.params[mix] = 1.0;
        p.tracks[i].inserts = vec![fx];
        p.tracks[i].volume_db = -4.0;
    };
    aux(p, "Hall Reverb", FxKind::Reverb, 3);
    aux(p, "Echo", FxKind::Delay, 4);
    let at =
        |p: &model::Project, name: &str| p.tracks.iter().position(|t| t.name == name).unwrap_or(0);
    let piano = at(p, "Classic Electric Piano");
    let grand = at(p, "Steinway Grand Piano");
    let pad = at(p, "Analog Pad");
    let drums = at(p, "Drummer");
    let bass = at(p, "Electric Bass");
    let vocal = at(p, "Lead Vocal");
    for (t, a, b) in [
        (piano, 0.45, 0.30),
        (grand, 0.35, 0.0),
        (pad, 0.60, 0.25),
        (bass, 0.20, 0.0),
        (vocal, 0.58, 0.42),
    ] {
        let mut v = vec![SendSlot {
            bus: Some(1),
            level: a,
            pre: t == pad,
        }];
        if b > 0.0 {
            v.push(SendSlot {
                bus: Some(2),
                level: b,
                pre: false,
            });
        }
        p.tracks[t].sends = v;
    }
    // A broad boost low and high, a notch in the middle.
    let mut eq = Insert::new(FxKind::ChannelEq);
    eq.params[..5].copy_from_slice(&[4.0, -3.5, 600.0, 1.4, 5.0]);
    p.tracks[piano].inserts = vec![Insert::new(FxKind::Delay), eq];
    let mut eq2 = Insert::new(FxKind::ChannelEq);
    eq2.params[..5].copy_from_slice(&[-2.0, 5.0, 2500.0, 0.9, 1.0]);
    let mut comp = Insert::new(FxKind::Compressor);
    comp.params[0] = -34.0;
    comp.params[1] = 6.0;
    p.tracks[grand].inserts = vec![eq2, comp];
    let mut multi = Insert::new(FxKind::MultiEq);
    multi.params.copy_from_slice(&[
        70.0, 3.0, 200.0, -4.0, 1.2, 700.0, 2.5, 1.5, 2200.0, -3.0, 2.0, 5500.0, 5.0, 1.0, 11000.0,
        -2.0,
    ]);
    p.tracks[pad].inserts = vec![Insert::new(FxKind::Chorus), multi];
    // A noise gate and a limiter on the vocal, and a group of the two pianos.
    let mut gate = Insert::new(FxKind::Gate);
    gate.params[0] = -38.0;
    let mut limiter = Insert::new(FxKind::Limiter);
    limiter.params[0] = 9.0;
    limiter.params[1] = -3.0;
    p.tracks[vocal].inserts = vec![gate, limiter];
    if let Some(g) = p.add_group("Keys") {
        p.assign_group(piano, Some(g));
        p.assign_group(grand, Some(g));
    }
    for (t, mode) in [
        (piano, AutoMode::Read),
        (grand, AutoMode::Read),
        (pad, AutoMode::Touch),
        (drums, AutoMode::Latch),
        (bass, AutoMode::Off),
    ] {
        p.tracks[t].auto.mode = mode;
    }
    for (beat, db) in [
        (0.0, -9.0),
        (16.0, -3.2),
        (32.0, -9.0),
        (48.0, -1.0),
        (64.0, -12.0),
    ] {
        p.tracks[piano].auto.set_point(AutoTarget::Volume, beat, db);
    }
    for (beat, pan) in [(0.0, -0.6), (24.0, 0.6), (48.0, -0.2)] {
        p.tracks[piano].auto.set_point(AutoTarget::Pan, beat, pan);
    }
    p.tracks[grand]
        .auto
        .set_point(AutoTarget::Volume, 0.0, -4.0);
    // The Tape Delay's feedback follows a lane with bent stretches.
    let feedback = AutoTarget::Fx { slot: 0, param: 1 };
    for (beat, v) in [(0.0, 0.2), (16.0, 0.8), (40.0, 0.35), (60.0, 0.9)] {
        p.tracks[piano].auto.set_point(feedback, beat, v);
    }
    p.tracks[piano].auto.fx[0].points[0].curve = 0.7;
    p.tracks[piano].auto.fx[0].points[1].curve = -0.6;
    p.tracks[piano].auto.shown = feedback;
    let (piano_t, grand_t) = (piano, grand);
    p.selected_track = grand_t;
    p.position_beats = 16.0;
    p.playing = true;
    app.ui.show_mixer = true;
    app.ui.bottom = Bottom::Mixer;
    match kind {
        "fx" => app.ui.mixer.fx_window = Some((grand_t, 1)),
        "eq" => app.ui.mixer.fx_window = Some((piano_t, 1)),
        "multi" => app.ui.mixer.fx_window = Some((pad, 1)),
        "limiter" => app.ui.mixer.fx_window = Some((vocal, 1)),
        "auto" => {
            app.ui.mixer.auto_editor.track = Some(piano_t);
            app.ui.mixer.auto_editor.span = 72.0;
        }
        _ => {}
    }
}

/// The window's class (Wayland: its app id). Wayland desktops find the icon and the menu entry
/// by the `.desktop` file's name, which is `ovrtone` for the packages and the Flatpak's
/// own id inside a Flatpak, so the class is the same; X11 desktops use `StartupWMClass`.
fn window_app_id() -> String {
    std::env::var("FLATPAK_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "ovrtone".to_string())
}

/// The main window's handle on Windows, which owns plugin editor windows.
fn main_window_handle(frame: &eframe::Frame) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match frame.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

/// Ends the program without the plugin files' unload code: some crash in it, with everything
/// already done.
fn leave_now(code: i32) -> ! {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn TerminateProcess(process: *mut std::ffi::c_void, code: u32) -> i32;
        }
        // SAFETY: both are plain Windows calls, and the handle is the one for this process.
        unsafe {
            TerminateProcess(GetCurrentProcess(), code as u32);
        }
    }
    std::process::exit(code)
}

fn main() -> eframe::Result {
    // Before anything reads a setting.
    recent_files::migrate_old_settings();
    // Automatic: from the physical cores the system reports (half of them, at least one).
    let workers = match settings::worker_limit() {
        0 => ovrtone_engine::parallel::default_workers(hardware::physical_cores()),
        n => n,
    };
    ovrtone_engine::Engine::set_worker_limit(workers);
    ovrtone_engine::set_plugin_block(settings::plugin_block());
    settings::load_view();
    #[cfg(windows)]
    ovrtone_host::set_isolation(settings::plugin_isolation());
    ovrtone_engine::set_capture_minutes(settings::capture_minutes());
    bench::mark_start();
    // Started by an update that is waiting for the program to close: put the update in place.
    if updater::run_helper_if_requested() {
        return Ok(());
    }
    let all_args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    // Started by the Plugins tab to read one plugin file, where a crash costs nothing.
    #[cfg(windows)]
    if let Some(block) = ovrtone_host::host_request(&all_args) {
        leave_now(ovrtone_host::run_plugin_host(block));
    }
    if let Some(file) = plugin_scan::scan_request(&all_args) {
        leave_now(plugin_scan::scan_and_print(&file));
    }
    // Started by the Stem Splitter to do its work, where a failure costs only the split.
    if let Some(request) = stem_worker::request(&all_args) {
        leave_now(stem_worker::run(&request));
    }
    let bench_out = bench::requested(&all_args);
    let shots_dir = bench::shots_requested(&all_args);
    if bench_out.is_none() && shots_dir.is_none() {
        updater::clean_up_after_update();
    }
    // Measuring runs leave the user's settings (recent projects, size) alone.
    let measuring = bench_out.is_some() || shots_dir.is_some();
    // No stock instruments: a track sounds only from what the user puts on it.
    if !measuring {
        ovrtone_engine::set_built_in_voices(false);
    }
    if !measuring {
        // In the theme's colours: the program opens looking as it was left.
        themes::load_chosen();
        let rgb = |c: egui::Color32| [c.r(), c.g(), c.b()];
        let colors = ovrtone_host::splash::SplashColors {
            background: rgb(theme::panel_bg()),
            text: rgb(theme::text()),
            dim: rgb(theme::text_dim()),
        };
        let logo = include_bytes!("../assets/brand/ovrtone-wordmark-black.png");
        if let Ok(logo) = eframe::icon_data::from_png_bytes(logo) {
            ovrtone_host::splash::show(
                &logo.rgba,
                (logo.width, logo.height),
                format!(
                    "Version {}   |   Bankz Sound Lab",
                    env!("CARGO_PKG_VERSION")
                ),
                colors,
            );
        }
    }
    // Release builds: on a crash, write a report to the temp folder instead of a raw panic.
    human_panic::setup_panic!(human_panic::Metadata::new(
        "Ovrtone",
        env!("CARGO_PKG_VERSION")
    ));
    init_logging();
    i18n::init();
    project_io::remove_stale_media(std::time::Duration::from_secs(7 * 24 * 3600));
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1600.0, 980.0])
        // Smaller than this the panels have no room left, even scaled down.
        .with_min_inner_size([800.0, 520.0])
        // Fill the screen, as Logic does; on a small screen 1600 x 980 would not fit.
        .with_maximized(true)
        // Just the app name, on every system: no project name or view in the title bar.
        .with_title("Ovrtone")
        // Window class: lets Linux desktops match the window to its .desktop entry and icon.
        .with_app_id(window_app_id())
        // Windows and Linux: no title bar from the system; the menu bar row has the window
        // buttons (see window_chrome.rs).
        .with_decorations(!window_chrome::CUSTOM);
    // Title bar / taskbar icon. A broken icon must not stop the app from starting.
    // macOS takes the Dock icon from the app bundle (ovrtone.icns, on the macOS icon tile);
    // setting one here would swap it for the bare logo once the app is running. An empty
    // icon tells eframe to leave the Dock icon alone instead of using its default egui logo.
    if cfg!(target_os = "macos") {
        viewport = viewport.with_icon(egui::IconData::default());
    } else {
        match eframe::icon_data::from_png_bytes(APP_ICON_PNG) {
            Ok(icon) => viewport = viewport.with_icon(icon),
            Err(e) => tracing::warn!("app icon could not be decoded: {e}"),
        }
    }
    let wgpu_options = graphics::config();
    let started_backends = graphics::started_backends(&wgpu_options);
    let opts = eframe::NativeOptions {
        viewport,
        wgpu_options,
        ..Default::default()
    };
    tracing::info!(
        "launch: {:.0} ms, making the window",
        bench::since_start_ms()
    );
    let result = eframe::run_native(
        "Ovrtone",
        opts,
        Box::new(|cc| {
            launch_step("Loading the interface", 0.1);
            themes::load_chosen();
            theme::load_type();
            theme::install_fonts(&cc.egui_ctx);
            theme::load_accent();
            theme::apply(&cc.egui_ctx);
            // Cmd/Ctrl + = / - / 0 resize the interface through `ui_scale` instead, which
            // also keeps the fit for small windows.
            cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);
            // Every start begins clean; only the layout pictures need something to show.
            let project = if shots_dir.is_some() {
                Project::demo()
            } else {
                Project::empty()
            };
            let mut audio_error = None;
            let mut midi_out = None;
            launch_step("Starting the audio engine", 0.15);
            // What was chosen in Settings, or the usual device if that will not start.
            let setup = settings::AudioSetup::load();
            let started = ovrtone_engine::Engine::start_chosen(
                &setup.system,
                &setup.device,
                setup.buffer,
                setup.rate,
            )
            .or_else(|e| {
                tracing::warn!("the audio setup did not start: {e}");
                ovrtone_engine::Engine::start()
            });
            let sync = match started {
                Ok(e) => {
                    let mut sync = engine_sync::EngineSync::new(e, &project);
                    if !measuring {
                        midi_out = sync
                            .take_midi_out()
                            .zip(sync.take_mtc_out())
                            .map(|(notes, mtc)| midi_out::MidiOut::start(notes, mtc));
                    }
                    Some(sync)
                }
                Err(err) => {
                    tracing::warn!("audio disabled: {err}");
                    audio_error =
                        Some(i18n::trf("No sound: {}. Check your audio device.", &[&err]));
                    None
                }
            };
            launch_step("Connecting MIDI", 0.25);
            let mut app = App {
                autosave: autosave::Autosave::default(),
                saved: file_commands::content(&project),
                history: undo::History::new(&project),
                project,
                ui: UiState::new(),
                sync,
                path: None,
                native_menu: None,
                bench: bench_out.clone().map(bench::Probe::new),
                shots: shots_dir.clone().map(bench::LayoutShots::new),
                frames_since_record: 0,
                midi: (!measuring).then(midi_in::MidiIn::new),
                midi_out,
                surface: None,
                surface_tried: None,
                mtc_in: (!measuring).then(|| mtc_in::MtcIn::new(cc.egui_ctx.clone())),
                chase: mtc_in::Chase::default(),
                updater: updater::Updater::new(),
            };
            if !measuring {
                app.autosave.look_for_recovery();
                app.ui.typing.on = settings::typing_at_start();
                app.ui.update_auto = updater::auto_enabled();
                app.updater.check_at_startup();
            }
            #[cfg(target_os = "macos")]
            {
                app.native_menu = Some(native_menu::NativeMenu::install(&cc.egui_ctx));
            }
            launch_step("Looking for plugins", 0.3);
            if !measuring {
                app.ui.plugins.clap.start(false, &cc.egui_ctx);
                // A plugin process started now, so the first plugin opens without waiting for one.
                ovrtone_host::warm_up();
                app.wait_for_launch_scan();
                app.ui.launching = true;
            }
            app.ui.status = audio_error;
            if let Some(n) = measuring
                .then(|| {
                    std::env::var("OVRTONE_BENCH_TRACKS")
                        .ok()?
                        .parse::<usize>()
                        .ok()
                })
                .flatten()
            {
                app.project = bench::many_tracks(n.clamp(1, 1000));
                app.saved = file_commands::content(&app.project);
                app.history.reset(&app.project);
            }
            if app.shots.is_some() {
                // The pictures also show the Tempo track, with a few tempo changes.
                app.ui.tracks.show_tempo = true;
                app.ui.tracks.show_markers = true;
                app.ui.tracks.show_signatures = true;
                app.project.set_signature_from(5, (3, 4));
                app.project.set_signature_from(9, (6, 8));
                // A VCA and an External MIDI track near the front, to see their strips.
                {
                    use model::{Track, TrackKind};
                    let pr = &mut app.project;
                    pr.tracks.insert(1, Track::new("VCA 1", TrackKind::Vca));
                    pr.vca_inserted(1);
                    let mut ext = Track::new("External MIDI 1", TrackKind::ExternalMidi);
                    ext.midi_port = Some("USB MIDI Interface".into());
                    ext.midi_channel = 3;
                    pr.tracks.insert(2, ext);
                    pr.vca_inserted(2);
                    pr.assign_vca(0, Some(1));
                    pr.assign_vca(3, Some(1));
                    pr.tracks[1].volume_db = -4.0;
                }
                if let Some(r) = app
                    .project
                    .tracks
                    .get_mut(4)
                    .and_then(|t| t.regions.get_mut(0))
                {
                    r.params.fade_in = 4.0;
                    r.params.fade_out = 8.0;
                }
                for (bar, name) in [(0.0, "Intro"), (4.0, "Verse"), (12.0, "Chorus")] {
                    let i = app.project.add_marker(app.project.bar_to_beats(bar));
                    app.project.markers[i].name = name.into();
                }
                app.project.add_tempo_change(32.0, 132.0);
                app.project.add_tempo_change(48.0, 104.0);
                // A tempo curve from the first change to the second.
                app.project.set_tempo_ramp(1, true);
                // `OVRTONE_SHOT_SCENE=pattern` or `drummer` opens that editor on a new region
                // of the Drummer track.
                let scene = std::env::var("OVRTONE_SHOT_SCENE").unwrap_or_default();
                if scene == "score" || scene == "midifx" {
                    app.ui.tracks.selected = Some((0, 0));
                    app.project.selected_track = 0;
                    app.ui.bottom = Bottom::PianoRoll;
                    app.ui.score_tab = true;
                    app.ui.show_midi_fx = scene == "midifx";
                    app.project.tracks[0].midi_fx = vec![
                        midi_fx::FxSlot {
                            fx: midi_fx::MidiFx::Arpeggiator {
                                rate: pattern::StepRate::Eighth,
                                pattern: midi_fx::ArpPattern::UpDown,
                                octaves: 2,
                                gate: 80,
                            },
                            on: true,
                        },
                        midi_fx::FxSlot {
                            fx: midi_fx::MidiFx::Scale {
                                root: 0,
                                minor: true,
                            },
                            on: true,
                        },
                    ];
                }
                if let Some(kind) = scene.strip_prefix("flex") {
                    shot_flex_scene(&mut app, &cc.egui_ctx, kind);
                }
                // The Spatial Renderer and a panner, with two objects in a 7.1.4 room on headphones.
                if scene == "spatial" {
                    use ovrtone_engine::{ObjectPlace, SpatialMode, SpeakerLayout};
                    let pr = &mut app.project;
                    pr.spatial_mode = SpatialMode::Binaural(SpeakerLayout::Surround714);
                    pr.tracks[0].object = Some(ObjectPlace {
                        azimuth: -40.0,
                        elevation: 20.0,
                        ..ObjectPlace::default()
                    });
                    if let Some(t) = pr.tracks.get_mut(1) {
                        t.object = Some(ObjectPlace {
                            azimuth: 120.0,
                            elevation: 50.0,
                            size: 0.4,
                            ..ObjectPlace::default()
                        });
                    }
                    pr.selected_track = 0;
                    app.ui.spatial.open = true;
                    app.ui.spatial.panner_open = true;
                }
                if let Some(kind) = scene.strip_prefix("mix") {
                    shot_mixer_scene(&mut app, kind);
                }
                if scene == "pattern" || scene == "drummer" {
                    let pr = &mut app.project;
                    if let Some(t) = pr
                        .tracks
                        .iter()
                        .position(|t| t.kind == model::TrackKind::Drummer)
                    {
                        if let Some(ri) = pr.add_generated_region(t, scene == "drummer", 40.0) {
                            pr.selected_track = t;
                            app.ui.tracks.selected = Some((t, ri));
                            app.ui.bottom = Bottom::PianoRoll;
                        }
                    }
                }
                app.saved = file_commands::content(&app.project);
                app.history.reset(&app.project);
            }
            if !measuring {
                app.ui.recent = recent_files::Recent::load();
                app.ui.ui_scale = ui_scale::load();
            }
            // `ovrtone song.ovrtone` opens a project; `ovrtone song.wav drums.aiff` imports each
            // file as an audio track (into the opened project, if any).
            // `--play` starts the transport on launch; `--piano` opens the Piano Roll.
            // A benchmark run takes no other arguments.
            let args: Vec<std::ffi::OsString> = if measuring {
                Vec::new()
            } else {
                std::env::args_os().skip(1).collect()
            };
            // Nothing asked for: the chooser, once the launch screen goes.
            if args.is_empty() && !measuring && settings::chooser_at_launch() {
                let has_recent = !app.ui.recent.paths().is_empty();
                app.ui.chooser = Some(start_screen::Chooser::at_launch(has_recent));
            }
            let is_project =
                |a: &std::ffi::OsString| ovrtone_project::is_project_file(std::path::Path::new(a));
            if let Some(file) = args.iter().find(|a| is_project(a)) {
                app.open(std::path::Path::new(file), &cc.egui_ctx);
            }
            for arg in args.iter().filter(|a| !is_project(a)) {
                if arg == "--play" {
                    app.project.playing = true;
                } else if arg == "--piano" {
                    app.ui.bottom = Bottom::PianoRoll;
                } else {
                    app.import(std::path::Path::new(arg), &cc.egui_ctx);
                }
            }
            app.load_plugins_on_launch_screen();
            Ok(Box::new(app))
        }),
    );
    // The app (and with it the engine's open media files) is gone by now.
    project_io::remove_session_media();
    // OpenGL can't draw to this window: the app never started, so try the system's own API.
    let step = std::env::var(graphics::STEP_VAR)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    if let Some(how) = result.as_ref().err().and_then(|e| {
        graphics::next_relaunch(
            step,
            started_backends,
            e,
            wayland,
            cfg!(target_os = "linux"),
        )
    }) {
        tracing::warn!("the window could not be drawn; relaunching ({how:?})");
        match graphics::relaunch(&how) {
            Ok(code) => std::process::exit(code),
            Err(e) => tracing::error!("could not relaunch: {e}"),
        }
    }
    if let (Some(out), Err(e)) = (&bench_out, &result) {
        bench::report_without_window(out, &e.to_string());
        return Ok(());
    }
    // The window has closed and the project is dealt with.
    if result.is_ok() {
        leave_now(0);
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_icon_decodes_as_a_square_rgba_image() {
        let icon = eframe::icon_data::from_png_bytes(super::APP_ICON_PNG).unwrap();
        assert_eq!((icon.width, icon.height), (256, 256));
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
    }
}

/// What opening another DAW's session could not carry over.
fn import_log_window(ctx: &egui::Context, s: &mut UiState) {
    let Some((name, log)) = s.import_log.as_ref() else {
        return;
    };
    let mut open = true;
    egui::Window::new(format!("Opened {name}"))
        .id(egui::Id::unique("import_log"))
        .open(&mut open)
        .default_width(460.0)
        .show(ctx, |ui| {
            if log.is_empty() {
                ui.label("Everything in the session was carried over.");
            } else {
                ui.label("Opened. These were not carried over:");
                egui::ScrollArea::vertical()
                    .max_height(300.0)
                    .show(ui, |ui| {
                        for line in log {
                            ui.label(format!("• {line}"));
                        }
                    });
            }
            ui.label(egui::RichText::new("Save it to keep it as an Ovrtone project.").weak());
        });
    if !open {
        s.import_log = None;
    }
}

/// A step of the launch: shown on the launch screen and timed in the log (`RUST_LOG=info`), so a
/// slow start shows where its time goes.
fn launch_step(words: &str, fraction: f32) {
    tracing::info!("launch: {:.0} ms, {words}", bench::since_start_ms());
    ovrtone_host::splash::step(words, fraction);
}
