mod scanner;
mod hasher;
mod dedupe;
#[allow(dead_code)]
mod preview;
mod cache;
mod settings;

// Build information - update these with each release
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

use dedupe::DuplicateGroup;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use gtk4::prelude::*;
use libadwaita::prelude::*;
use glib::clone;

struct ProgressState {
    done:         AtomicUsize,
    total:        AtomicUsize,
    failed:       AtomicUsize,
    is_hashing:   AtomicBool,
    dirty:        AtomicBool,
}

impl ProgressState {
    fn new() -> Self {
        ProgressState {
            done:         AtomicUsize::new(0),
            total:        AtomicUsize::new(0),
            failed:       AtomicUsize::new(0),
            is_hashing:   AtomicBool::new(true),
            dirty:        AtomicBool::new(false),
        }
    }
}

fn main() {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let worker_threads = (cpus - 1).max(1);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(worker_threads)
        .build_global();

    let app = libadwaita::Application::builder()
        .application_id("com.imphash.app")
        .build();

    app.connect_activate(|app| {
        build_ui(app);
    });

    app.run();
}

fn btn_icon_text(label: &str, icon: &str) -> gtk4::Button {
    let btn = gtk4::Button::new();
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let img = gtk4::Image::from_icon_name(icon);
    img.set_pixel_size(16);
    let lbl = gtk4::Label::new(Some(label));
    hbox.append(&img);
    hbox.append(&lbl);
    btn.set_child(Some(&hbox));
    btn.set_has_frame(true);
    btn
}

fn add_dir_row(
    path: &str,
    is_ref: bool,
    dirs: &Arc<Mutex<Vec<String>>>,
    ref_dirs: &Arc<Mutex<HashSet<String>>>,
    dir_list: &gtk4::ListBox,
    scan_btn: &gtk4::Button,
    rdata2: &Arc<Mutex<Vec<GroupData>>>,
    auto_save: &Arc<AtomicBool>,
    rotation_enabled: &Arc<AtomicBool>,
    threshold_val: &Arc<AtomicU32>,
    select_by_btn: &gtk4::MenuButton,
) {
    let mut d = dirs.lock().unwrap();
    if d.contains(&path.to_string()) { return; }
    d.push(path.to_string());
    drop(d);

    let row = gtk4::ListBoxRow::new();
    let hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let lbl = gtk4::Label::new(Some(path));
    lbl.set_hexpand(true);
    lbl.set_halign(gtk4::Align::Start);
    let ref_cb = gtk4::CheckButton::with_label("Reference");

    let dir_refs = ref_dirs.clone();
    let dtext = path.to_string();
    let dir_lbl = lbl.clone();
    let rd_for_refresh = rdata2.clone();
    let rd_for_refresh2 = ref_dirs.clone();
    let as3 = auto_save.clone();
    let rot3 = rotation_enabled.clone();
    let thr3 = threshold_val.clone();
    let dirs3 = dirs.clone();
    let refs4 = ref_dirs.clone();
    let sbb = select_by_btn.clone();
    let dir_list2 = dir_list.clone();
    ref_cb.connect_toggled(move |cb| {
        if cb.is_active() {
            let mut rd = dir_refs.lock().unwrap();
            rd.clear();
            rd.insert(dtext.clone());
            drop(rd);
            let mut child = dir_list2.first_child();
            while let Some(widget) = child {
                if let Some(list_row) = widget.downcast_ref::<gtk4::ListBoxRow>() {
                    if let Some(box_child) = list_row.child() {
                        if let Some(hbox) = box_child.downcast_ref::<gtk4::Box>() {
                            let mut hc = hbox.first_child();
                            while let Some(w) = hc {
                                if let Some(other_cb) = w.downcast_ref::<gtk4::CheckButton>() {
                                    if other_cb != cb && other_cb.is_active() && other_cb.label().as_deref() == Some("Reference") {
                                        other_cb.set_active(false);
                                    }
                                }
                                hc = w.next_sibling();
                            }
                        }
                    }
                }
                child = widget.next_sibling();
            }
            dir_lbl.set_css_classes(&["ref-path"]);
            sbb.set_visible(false);
        } else {
            let mut rd = dir_refs.lock().unwrap();
            rd.remove(&dtext);
            dir_lbl.set_css_classes(&[]);
            sbb.set_visible(rd.is_empty());
            drop(rd);
        }
        refresh_all_ref_styling(&rd_for_refresh, &rd_for_refresh2);
        save_settings(&as3, &rot3, &thr3, &*dirs3.lock().unwrap(), &*refs4.lock().unwrap());
    });
    if is_ref { ref_cb.set_active(true); }

    let remove_btn = gtk4::Button::with_label("\u{2715}");
    remove_btn.set_css_classes(&["circular", "flat"]);
    let dirs2 = dirs.clone();
    let dir_refs2 = ref_dirs.clone();
    let dir_list2 = dir_list.clone();
    let scan2 = scan_btn.clone();
    let lbl2 = lbl.clone();
    let row2 = row.clone();
    let as2 = auto_save.clone();
    let rot2 = rotation_enabled.clone();
    let thr2 = threshold_val.clone();
    let dirs5 = dirs.clone();
    let refs3 = ref_dirs.clone();
    remove_btn.connect_clicked(move |_| {
        let t = lbl2.text().to_string();

        {
            let mut d = dirs2.lock().unwrap();
            d.retain(|x| *x != t);
            if d.is_empty() { scan2.set_sensitive(false); }
        } // dirs lock released here

        {
            let mut rd = dir_refs2.lock().unwrap();
            rd.remove(&t);
        } // ref_dirs lock released here

        dir_list2.remove(&row2);
        if dir_list2.row_at_index(0).is_none() {
            dir_list2.set_visible(false);
        }

        save_settings(&as2, &rot2, &thr2, &*dirs5.lock().unwrap(), &*refs3.lock().unwrap());
    });

    hbox.append(&lbl);
    hbox.append(&ref_cb);
    hbox.append(&remove_btn);
    row.set_child(Some(&hbox));
    dir_list.append(&row);
    dir_list.set_visible(true);
    scan_btn.set_sensitive(true);
}

fn save_settings(
    auto_save: &AtomicBool,
    rotation_enabled: &AtomicBool,
    threshold_val: &AtomicU32,
    dirs: &[String],
    ref_dirs: &HashSet<String>,
) {
    if auto_save.load(Ordering::Relaxed) {
        let rot = rotation_enabled.load(Ordering::Relaxed);
        let thr = threshold_val.load(Ordering::Relaxed);
        settings::Settings {
            rotation_enabled: rot,
            auto_save: true,
            threshold: thr,
            directories: dirs.to_vec(),
            ref_dirs: ref_dirs.iter().cloned().collect(),
        }.save();
    }
}

fn build_ui(app: &libadwaita::Application) {
    let cache = Arc::new(cache::HashCache::new().expect("Failed to init hash cache"));
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    let selection: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let dirs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let ref_dirs: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let per_file_refs: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let results_data: Arc<Mutex<Vec<GroupData>>> = Arc::new(Mutex::new(Vec::new()));
    let app_settings = settings::Settings::load();
    let rotation_enabled = Arc::new(AtomicBool::new(app_settings.rotation_enabled));
    let auto_save = Arc::new(AtomicBool::new(app_settings.auto_save));
    let threshold_val = Arc::new(AtomicU32::new(app_settings.threshold));

    let provider = gtk4::CssProvider::new();
    provider.load_from_string(
        ".ref-row { background: alpha(@accent_color, 0.08); border-left: 3px solid @accent_color; padding-left: 3px; }\
         .ref-image { border: 3px solid @accent_color; border-radius: 6px; }\
         .ref-path { color: @accent_color; font-weight: 600; }\
         .dir-list { border: 1px solid @borders; border-radius: 6px; background: @card_bg_color; }\
         .error { color: @error_color; font-weight: bold; }\
         .deleted { background: alpha(@error_color, 0.08); border-left: 3px solid @error_color; padding-left: 3px; }\
         .deleted label { color: @error_color; text-decoration: line-through; }\
         .deleted .dim-label { color: alpha(@error_color, 0.6); }\
         .moved { background: alpha(@success_color, 0.06); border-left: 3px solid @success_color; padding-left: 3px; }\
         .moved label { color: @success_color; }\
         .group-frame { border: 1px solid @borders; border-radius: 8px; background: @card_bg_color; transition: all 150ms ease; }\
         .group-frame:hover { border-color: alpha(@accent_color, 0.3); box-shadow: 0 1px 4px alpha(black, 0.08); }\
         .result-row { padding: 4px 6px; padding-left: 9px; border-radius: 4px; transition: background 100ms ease; }\
         .result-row:hover { background: alpha(@accent_color, 0.04); }\
         .result-row:selected { background: alpha(@accent_color, 0.1); }\
         .toolbar-box { background: @card_bg_color; border: 1px solid @borders; border-radius: 8px; padding: 6px 8px; }\
         .progress trough { min-height: 8px; border-radius: 4px; }\
         .progress progress { border-radius: 4px; }\
         .group-header-btn { margin: 0 2px; }\
         .group-header { background: alpha(@accent_color, 0.03); border-bottom: 1px solid @borders; padding: 4px 0; }\
         .column-header { font-weight: 600; font-size: 0.85em; color: @insensitive_fg_color; padding: 2px 6px; }\
         .col-header-row { border-bottom: 1px solid alpha(@borders, 0.7); margin-bottom: 2px; padding-bottom: 2px; }\
         .status-pill { background: alpha(@accent_color, 0.08); border-radius: 12px; padding: 2px 10px; font-weight: 600; }\
         .status-pill-ref { background: alpha(@accent_color, 0.15); color: @accent_color; border-radius: 12px; padding: 2px 10px; font-weight: 600; font-size: 0.85em; }\
         .status-pill-rot { background: alpha(@warning_color, 0.15); color: @warning_color; border-radius: 12px; padding: 2px 10px; font-weight: 600; font-size: 0.85em; }\
         viewport, scrolledwindow, list, box { border: none; background: transparent; }\
         .card { background: @card_bg_color; border: 1px solid @borders; border-radius: 8px; padding: 8px; }\
         .preview-separator { margin: 8px 0; }\
         .group-count-badge { background: alpha(@accent_color, 0.1); border-radius: 10px; padding: 1px 8px; font-size: 0.8em; color: @accent_color; font-weight: 600; }\
         .filter-active { background: @accent_color; color: white; border-radius: 4px; }"
    );
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(&display, &provider, gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }

    // --- Window ---
    let window = libadwaita::ApplicationWindow::builder()
        .application(app)
        .default_width(960)
        .default_height(720)
        .icon_name("imphash")
        .title("ImpHash - Duplicate Image Finder")
        .build();

    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    main_box.set_margin_start(12);
    main_box.set_margin_end(12);
    main_box.set_margin_top(12);
    main_box.set_margin_bottom(12);
    let header = libadwaita::HeaderBar::new();
    let outer_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    outer_box.append(&header);
    outer_box.append(&main_box);
    window.set_content(Some(&outer_box));

    // --- Create all widgets FIRST (before signals) ---

    let add_btn = btn_icon_text("Add Directory", "list-add-symbolic");
    add_btn.set_tooltip_text(Some("Add a directory to scan for duplicates"));

    let dir_list = gtk4::ListBox::new();
    dir_list.set_css_classes(&["dir-list"]);
    dir_list.set_visible(false);

    let scan_btn = btn_icon_text("Scan", "media-playback-start-symbolic");
    scan_btn.set_css_classes(&["suggested-action"]);
    scan_btn.set_tooltip_text(Some("Start scanning selected directories for duplicates"));
    let cancel_btn = btn_icon_text("Cancel", "process-stop-symbolic");
    cancel_btn.set_sensitive(false);
    cancel_btn.set_tooltip_text(Some("Cancel the current scan"));
    let pause_btn = btn_icon_text("Pause", "media-playback-pause-symbolic");
    pause_btn.set_sensitive(false);
    pause_btn.set_tooltip_text(Some("Pause or resume the current scan"));

    let progress_bar = gtk4::ProgressBar::new();
    progress_bar.set_css_classes(&["progress"]);
    progress_bar.set_show_text(true);
    progress_bar.set_height_request(32);
    progress_bar.set_hexpand(true);
    progress_bar.set_valign(gtk4::Align::Center);
    let status_label = gtk4::Label::new(None);
    status_label.set_halign(gtk4::Align::Start);
    status_label.set_valign(gtk4::Align::Start);
    status_label.set_margin_start(12);
    status_label.set_wrap(true);
    status_label.set_hexpand(true);
    status_label.set_max_width_chars(80);

    let stats_label = gtk4::Label::new(None);
    stats_label.set_css_classes(&["success"]);
    stats_label.set_halign(gtk4::Align::Start);
    stats_label.set_hexpand(true);
    let select_all_btn = btn_icon_text("Select All", "edit-select-all-symbolic");
    select_all_btn.set_tooltip_text(Some("Select all duplicate groups"));
    let clear_sel_btn = btn_icon_text("Clear", "edit-clear-symbolic");
    clear_sel_btn.set_tooltip_text(Some("Clear the current selection"));
    let move_sel_btn = btn_icon_text("Move Selected", "go-jump-symbolic");
    move_sel_btn.set_sensitive(false);
    move_sel_btn.set_tooltip_text(Some("Move selected files to a chosen folder"));
    let trash_sel_btn = btn_icon_text("Trash Selected", "user-trash-symbolic");
    trash_sel_btn.set_css_classes(&["destructive-action"]);
    trash_sel_btn.set_sensitive(false);
    trash_sel_btn.set_tooltip_text(Some("Move selected files to trash"));
    let invert_sel_btn = btn_icon_text("Invert Selection", "object-flip-horizontal-symbolic");
    invert_sel_btn.set_tooltip_text(Some("Invert the current selection"));
    let select_by_btn = gtk4::MenuButton::new();
    let sbb_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let sbb_img = gtk4::Image::from_icon_name("view-list-symbolic");
    sbb_img.set_pixel_size(16);
    let sbb_lbl = gtk4::Label::new(Some("Select By"));
    sbb_box.append(&sbb_img);
    sbb_box.append(&sbb_lbl);
    select_by_btn.set_child(Some(&sbb_box));
    select_by_btn.set_has_frame(true);
    let options_btn = gtk4::MenuButton::new();
    options_btn.set_label("Options");
    let clear_results_btn = btn_icon_text("Clear", "edit-clear-symbolic");
    clear_results_btn.set_tooltip_text(Some("Remove all results and clear the display"));

    let results_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    results_box.set_margin_top(6);
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_child(Some(&results_box));

    let no_results_label = gtk4::Label::new(Some("No duplicate images found"));
    no_results_label.set_css_classes(&["large-title"]);
    no_results_label.set_halign(gtk4::Align::Center);
    no_results_label.set_valign(gtk4::Align::Center);
    no_results_label.set_vexpand(true);
    no_results_label.set_visible(false);

    // --- Layout ---

    let dir_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    dir_row.append(&add_btn);
    dir_row.append(&options_btn);
    dir_row.append(&clear_results_btn);
    main_box.append(&dir_row);

    main_box.append(&dir_list);

    let scan_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    scan_row.append(&scan_btn);
    scan_row.append(&pause_btn);
    scan_row.append(&cancel_btn);
    main_box.append(&scan_row);

    let prog_row = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    prog_row.append(&progress_bar);
    prog_row.append(&status_label);
    main_box.append(&prog_row);

    let toolbar_revealer = gtk4::Revealer::new();
    toolbar_revealer.set_reveal_child(false);
    let toolbar_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    toolbar_box.set_css_classes(&["toolbar-box"]);
    toolbar_box.set_margin_top(4);
    toolbar_box.set_margin_bottom(4);
    let sel_group = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    sel_group.append(&select_all_btn);
    sel_group.append(&clear_sel_btn);
    sel_group.append(&invert_sel_btn);
    sel_group.append(&select_by_btn);
    let action_group = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    action_group.append(&move_sel_btn);
    action_group.append(&trash_sel_btn);
    let sep_toolbar = gtk4::Separator::new(gtk4::Orientation::Vertical);
    sep_toolbar.set_margin_start(4);
    sep_toolbar.set_margin_end(4);
    toolbar_box.append(&stats_label);
    toolbar_box.append(&sep_toolbar);
    toolbar_box.append(&sel_group);
    let sep_toolbar2 = gtk4::Separator::new(gtk4::Orientation::Vertical);
    sep_toolbar2.set_margin_start(4);
    sep_toolbar2.set_margin_end(4);
    toolbar_box.append(&sep_toolbar2);
    toolbar_box.append(&action_group);
    toolbar_revealer.set_child(Some(&toolbar_box));
    main_box.append(&toolbar_revealer);

    main_box.append(&scrolled);
    main_box.append(&no_results_label);

    // --- Populate directories from saved settings ---
    for dir in &app_settings.directories {
        let is_ref = app_settings.ref_dirs.contains(dir);
        add_dir_row(dir, is_ref, &dirs, &ref_dirs, &dir_list,
            &scan_btn, &results_data, &auto_save, &rotation_enabled, &threshold_val, &select_by_btn);
    }
    select_by_btn.set_visible(ref_dirs.lock().unwrap().is_empty());

    // --- Select By popover ---
    let popover = gtk4::PopoverMenu::from_model(None::<&gtk4::gio::MenuModel>);
    let popover_grid = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    popover_grid.set_margin_start(12);
    popover_grid.set_margin_end(12);
    popover_grid.set_margin_top(12);
    popover_grid.set_margin_bottom(12);
    let popover_title = gtk4::Label::new(Some("Select Files By"));
    popover_title.set_css_classes(&["heading"]);
    popover_title.set_halign(gtk4::Align::Start);
    popover_grid.append(&popover_title);

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_margin_bottom(4);
    popover_grid.append(&sep);

    let make_btn = |label: &str| {
        let btn = gtk4::Button::with_label(label);
        btn.set_size_request(80, -1);
        btn.set_has_frame(true);
        btn
    };

    // Popover rows — closures need their own captures
    let add_size_row = |label: &str, mode_base: i32, big_label: &str, small_label: &str, show_cancel: bool| {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let lbl = gtk4::Label::new(Some(label));
        lbl.set_width_chars(14);
        lbl.set_halign(gtk4::Align::Start);
        lbl.set_css_classes(&["dim-label"]);
        let big = make_btn(big_label);
        let small = make_btn(small_label);
        let small_for_big = small.clone();
        let big1 = big.clone();
        big1.clone().connect_clicked(clone!(#[strong] selection, #[strong] results_data,
            #[strong] status_label, move |_| {
            small_for_big.set_css_classes(&[]);
            big1.set_css_classes(&["filter-active"]);
            let data = results_data.lock().unwrap();
            apply_select_by(&data, &selection, mode_base, &status_label);
        }));
        let big_for_small = big.clone();
        let small1 = small.clone();
        small1.clone().connect_clicked(clone!(#[strong] selection, #[strong] results_data,
            #[strong] status_label, move |_| {
            big_for_small.set_css_classes(&[]);
            small1.set_css_classes(&["filter-active"]);
            let data = results_data.lock().unwrap();
            apply_select_by(&data, &selection, mode_base + 1, &status_label);
        }));
        row.append(&lbl);
        row.append(&big);
        row.append(&small);
        if show_cancel {
            let cancel = make_btn("Cancel");
            let cancel_big = big.clone();
            let cancel_small = small.clone();
            cancel.connect_clicked(clone!(#[strong] selection, #[strong] results_data,
                #[strong] stats_label, #[weak] move_sel_btn, #[weak] trash_sel_btn, move |_| {
                cancel_big.set_css_classes(&[]);
                cancel_small.set_css_classes(&[]);
                selection.lock().unwrap().clear();
                let data = results_data.lock().unwrap();
                for gd in data.iter() {
                    for fd in gd.files.iter() {
                        fd.check.set_active(false);
                    }
                }
                move_sel_btn.set_sensitive(false);
                trash_sel_btn.set_sensitive(false);
                stats_label.set_text("");
            }));
            row.append(&cancel);
        }
        row
    };

    popover_grid.append(&add_size_row("Image Size", 0, "Big", "Small", true));
    popover_grid.append(&add_size_row("Image Resolution", 2, "Big", "Small", true));
    popover_grid.append(&add_size_row("Path Length", 4, "Long", "Short", true));

    popover.set_child(Some(&popover_grid));
    select_by_btn.set_popover(Some(&popover));

    // --- Signals ---

    {
        let d = dirs.clone();
        let rd = ref_dirs.clone();
        let dl = dir_list.clone();
        let sb = scan_btn.clone();
        let sl = status_label.clone();
        let rdata = results_data.clone();
        let win = window.clone();
        let as_clone = auto_save.clone();
        let rot_clone = rotation_enabled.clone();
        let thr_clone = threshold_val.clone();
        let sbb = select_by_btn.clone();
        add_btn.connect_clicked(move |_| {
            let dialog = gtk4::FileDialog::new();
            dialog.set_title("Select directory to add");
            let dirs = d.clone();
            let ref_dirs = rd.clone();
            let dir_list = dl.clone();
            let scan_btn = sb.clone();
            let status_label = sl.clone();
            let rdata2 = rdata.clone();
            let as2 = as_clone.clone();
            let rot2 = rot_clone.clone();
            let thr2 = thr_clone.clone();
            let dirs2 = dirs.clone();
            let refs2 = ref_dirs.clone();
            let select_by_btn = sbb.clone();
            dialog.select_folder(
                Some(&win),
                None::<&gtk4::gio::Cancellable>,
                move |result| {
                    let path = match result {
                        Ok(f) => f.path().map(|p| p.to_string_lossy().to_string()),
                        Err(_) => None,
                    };
                    let path = match path {
                        Some(p) => p,
                        None => return,
                    };
                    add_dir_row(&path, false, &dirs, &ref_dirs, &dir_list,
                        &scan_btn, &rdata2, &as2, &rot2, &thr2, &select_by_btn);
                    save_settings(&as2, &rot2, &thr2, &*dirs2.lock().unwrap(), &*refs2.lock().unwrap());
                    status_label.set_text("");
                },
            );
        });
    }

    cancel_btn.connect_clicked(clone!(#[strong] cancel_flag, #[strong] window, move |_| {
        let confirm = gtk4::AlertDialog::builder()
            .message("Cancel scan?")
            .detail("Are you sure you want to cancel the current scan? Progress will be lost.")
            .buttons(["Cancel Scan", "Continue Scanning"])
            .build();
        let cf = cancel_flag.clone();
        confirm.choose(Some(&window), None::<&gtk4::gio::Cancellable>, move |result| {
            if matches!(result, Ok(0)) {
                cf.store(true, Ordering::Relaxed);
            }
        });
    }));

    pause_btn.connect_clicked(clone!(#[strong] pause_flag, move |btn| {
        let is_paused = pause_flag.load(Ordering::Relaxed);
        pause_flag.store(!is_paused, Ordering::Relaxed);
        if is_paused {
            btn.set_label("Pause");
        } else {
            btn.set_label("Resume");
        }
    }));

    {
        let sel_clone = selection.clone();
        let rd_clone = results_data.clone();
        let ref_dirs_clone = ref_dirs.clone();
        let msel = move_sel_btn.clone();
        let tsel = trash_sel_btn.clone();
        let sl = stats_label.clone();
        let slbl = status_label.clone();
        let dirs = dirs.clone();
        let dir_list = dir_list.clone();
        let window = window.clone();
        select_all_btn.connect_clicked(move |_| {
            if ref_dirs_clone.lock().unwrap().is_empty() {
                let dialog = libadwaita::AlertDialog::builder()
                    .heading("No reference directory configured")
                    .body("All files will be selected because no reference directory is set. What would you like to do?")
                    .build();
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("continue", "Continue");
                dialog.add_response("select_ref", "Select Reference Path");
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");
                let rd = rd_clone.clone();
                let sel = sel_clone.clone();
                let msel2 = msel.clone();
                let tsel2 = tsel.clone();
                let sl2 = sl.clone();
                let slbl2 = slbl.clone();
                let dirs2 = dirs.clone();
                let ref_dirs2 = ref_dirs_clone.clone();
                let dir_list2 = dir_list.clone();
                let win2 = window.clone();
                let win2b = win2.clone();
                dialog.connect_response(None, move |_, response| {
                    match response {
                        "select_ref" => {
                            let dir_list_snapshot: Vec<String> = {
                                let d = dirs2.lock().unwrap();
                                d.clone()
                            };
                            if dir_list_snapshot.is_empty() {
                                let info = libadwaita::AlertDialog::builder()
                                    .heading("No directories added")
                                    .body("Add directories using the \"Add Directory\" button first, then mark one as Reference.")
                                    .build();
                                info.add_response("ok", "OK");
                                info.set_default_response(Some("ok"));
                                info.set_close_response("ok");
                                info.present(Some(&win2b));
                            } else {
                                let str_refs: Vec<&str> = dir_list_snapshot.iter().map(|s| s.as_str()).collect();
                                let string_list = gtk4::StringList::new(&str_refs);
                                let dropdown = gtk4::DropDown::new(Some(string_list), None::<&gtk4::Expression>);
                                dropdown.set_hexpand(true);
                                dropdown.set_margin_top(8);
                                dropdown.set_margin_bottom(8);

                                let pick_dialog = libadwaita::AlertDialog::builder()
                                    .heading("Select Reference Directory")
                                    .extra_child(&dropdown)
                                    .build();
                                pick_dialog.add_response("cancel", "Cancel");
                                pick_dialog.add_response("select", "Select");
                                pick_dialog.set_response_appearance("select", libadwaita::ResponseAppearance::Suggested);
                                pick_dialog.set_default_response(Some("select"));
                                pick_dialog.set_close_response("cancel");

                                let dir_list3 = dir_list2.clone();
                                let slbl3 = slbl2.clone();
                                let rd3 = rd.clone();
                                let sel3 = sel.clone();
                                let msel3 = msel2.clone();
                                let tsel3 = tsel2.clone();
                                let sl3 = sl2.clone();
                                let dir_snap2 = dir_list_snapshot.clone();
                                let ref_dirs3 = ref_dirs2.clone();
                                let rd_for_refresh = rd3.clone();
                                let rd_for_refresh2 = ref_dirs3.clone();
                                pick_dialog.connect_response(None, move |_, response| {
                                    if response != "select" { return; }
                                    let idx = dropdown.selected() as usize;
                                    if idx >= dir_snap2.len() { return; }
                                    let path = &dir_snap2[idx];
                                    ref_dirs3.lock().unwrap().insert(path.clone());
                                    refresh_all_ref_styling(&rd_for_refresh, &rd_for_refresh2);
                                    let mut i = 0;
                                    while let Some(r) = dir_list3.row_at_index(i) {
                                        if let Some(child) = r.child() {
                                            if let Ok(hbox) = child.downcast::<gtk4::Box>() {
                                                if let Some(lbl_widget) = hbox.first_child() {
                                                    if let Ok(lbl) = lbl_widget.clone().downcast::<gtk4::Label>() {
                                                        if lbl.text() == *path {
                                                            if let Some(next) = lbl_widget.next_sibling() {
                                                                if let Ok(cb) = next.downcast::<gtk4::CheckButton>() {
                                                                    cb.set_active(true);
                                                                }
                                                                lbl_widget.set_css_classes(&["ref-path"]);
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        i += 1;
                                    }
                                    slbl3.set_text("Reference directory selected");
                                    do_select_all(&rd3, &sel3, &ref_dirs3, &msel3, &tsel3, &sl3);
                                });
                                pick_dialog.present(Some(&win2b));
                            }
                        }
                        "continue" => {
                            // Continue — show second warning
                            let confirm = libadwaita::AlertDialog::builder()
                                .heading("Are you sure?")
                                .body("No reference directory is set. All files will be selected. Do you want to continue?")
                                .build();
                            confirm.add_response("cancel", "Cancel");
                            confirm.add_response("continue2", "Continue");
                            confirm.set_default_response(Some("cancel"));
                            confirm.set_close_response("cancel");
                            let rd4 = rd.clone();
                            let sel4 = sel.clone();
                            let ref_dirs4 = ref_dirs2.clone();
                            let msel4 = msel2.clone();
                            let tsel4 = tsel2.clone();
                            let sl4 = sl2.clone();
                            confirm.connect_response(None, move |_, response| {
                                if response == "continue2" {
                                    do_select_all(&rd4, &sel4, &ref_dirs4, &msel4, &tsel4, &sl4);
                                }
                            });
                            confirm.present(Some(&win2b));
                        }
                        _ => {}
                    }
                });
                dialog.present(Some(&win2));
            } else {
                do_select_all(&rd_clone, &sel_clone, &ref_dirs_clone, &msel, &tsel, &sl);
            }
        });
    }

    clear_sel_btn.connect_clicked(clone!(#[strong] selection, #[strong] results_data,
        #[weak] move_sel_btn, #[weak] trash_sel_btn, #[weak] stats_label, move |_| {
        selection.lock().unwrap().clear();
        let data = results_data.lock().unwrap();
        for gd in data.iter() {
            for fd in gd.files.iter() {
                fd.check.set_active(false);
            }
        }
        move_sel_btn.set_sensitive(false);
        trash_sel_btn.set_sensitive(false);
        stats_label.set_text("");
    }));

    invert_sel_btn.connect_clicked(clone!(#[strong] selection, #[strong] results_data,
        #[weak] move_sel_btn, #[weak] trash_sel_btn, #[weak] stats_label, move |_| {
        let data = results_data.lock().unwrap();
        for gd in data.iter() {
            for fd in gd.files.iter() {
                fd.check.set_active(!fd.check.is_active());
            }
        }
        drop(data);
        let n = selection.lock().unwrap().len();
        move_sel_btn.set_sensitive(n > 0);
        trash_sel_btn.set_sensitive(n > 0);
        stats_label.set_text(&format!("Selected: {}", n));
    }));

    trash_sel_btn.connect_clicked(clone!(#[strong] selection, #[strong] status_label, 
        #[strong] window, #[strong] results_data, move |_| {
        let paths: Vec<String> = selection.lock().unwrap().iter().cloned().collect();
        let n = paths.len();
        if n == 0 { return; }

        let dialog = libadwaita::AlertDialog::builder()
            .heading("Move to Trash?")
            .body(format!(
                "{} {} will be moved to trash.",
                n, if n == 1 { "file" } else { "files" }
            ))
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("trash", "Move to Trash");
        dialog.set_response_appearance("trash", libadwaita::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");

        let sel = selection.clone();
        let sl = status_label.clone();
        let rd = results_data.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "trash" { return; }
            let paths_to_trash: Vec<String> = sel.lock().unwrap().iter().cloned().collect();
            let mut ok = 0usize;
            for p in &paths_to_trash {
                if let Err(e) = trash::delete(p) {
                    sl.set_text(&format!("Failed to trash {}: {}", p, e));
                } else {
                    ok += 1;
                    let data = rd.lock().unwrap();
                    for group in data.iter() {
                        for file in group.files.iter() {
                            if file.path == *p {
                                file.deleted_label.set_visible(true);
                                file.row.set_css_classes(&["deleted"]);
                            }
                        }
                    }
                }
            }
            sel.lock().unwrap().clear();
            if ok == paths_to_trash.len() {
                sl.set_text(&format!("Moved {} {} to trash", ok, if ok == 1 { "file" } else { "files" }));
            }
        });
        dialog.present(Some(&window));
    }));

    move_sel_btn.connect_clicked(clone!(#[strong] selection, #[strong] status_label,
        #[strong] window, move |_| {
        let paths: Vec<String> = selection.lock().unwrap().iter().cloned().collect();
        if paths.is_empty() { return; }

        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Select destination folder");
        let paths2 = paths.clone();
        let sl = status_label.clone();
        let sel = selection.clone();
        let win = window.clone();
        dialog.select_folder(
            Some(&window),
            None::<&gtk4::gio::Cancellable>,
            move |result| {
                if let Ok(file) = result {
                    if let Some(dest) = file.path() {
                        let n = paths2.len();
                        let confirm = gtk4::AlertDialog::builder()
                            .message("Move selected files?")
                            .detail(&format!("This will move {} file(s) to:\n{}", n, dest.display()))
                            .buttons(["Move", "Cancel"])
                            .build();
                        
                        let paths3 = paths2.clone();
                        let sl2 = sl.clone();
                        let sel2 = sel.clone();
                        let dest2 = dest.clone();
                        confirm.choose(Some(&win), None::<&gtk4::gio::Cancellable>, move |confirm_result| {
                            if !matches!(confirm_result, Ok(0)) { return; }
                            
                            let mut ok = 0usize;
                            for p in &paths3 {
                                let src = std::path::Path::new(p);
                                let name = src.file_name().unwrap_or_default();
                                let target = dest2.join(name);
                                // Try rename first (fast, same filesystem),
                                // fall back to copy + remove.
                                if std::fs::rename(p, &target).is_ok() {
                                    ok += 1;
                                } else if std::fs::copy(p, &target).is_ok()
                                    && std::fs::remove_file(p).is_ok()
                                {
                                    ok += 1;
                                } else {
                                    sl2.set_text(&format!("Failed to move: {}", p));
                                }
                            }
                            sel2.lock().unwrap().clear();
                            if ok == paths3.len() {
                                sl2.set_text(&format!("Moved {} file(s) to {}", paths3.len(), dest2.display()));
                            }
                        });
                    }
                }
            },
        );
    }));

    // --- Options popover ---
    let options_popover = gtk4::Popover::new();
    let options_grid = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    options_grid.set_margin_start(8);
    options_grid.set_margin_end(8);
    options_grid.set_margin_top(8);
    options_grid.set_margin_bottom(8);
    let options_title = gtk4::Label::new(Some("Options"));
    options_title.set_css_classes(&["heading"]);
    options_grid.append(&options_title);

    let del_cache_btn = btn_icon_text("Delete cached database", "edit-delete-symbolic");
    del_cache_btn.set_tooltip_text(Some("Remove all cached image hashes"));
    del_cache_btn.connect_clicked(clone!(#[strong] cache, #[strong] window, #[strong] options_popover, #[strong] status_label, move |_| {
        options_popover.popdown();
        let dialog = libadwaita::AlertDialog::builder()
            .heading("Delete cached hash database?")
            .body("This will remove all cached image hashes. They will be recomputed on the next scan. This cannot be undone.")
            .build();
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("delete", "Delete");
        dialog.set_response_appearance("delete", libadwaita::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let cache = cache.clone();
        let status_label = status_label.clone();
        dialog.connect_response(None, move |_, response| {
            if response != "delete" { return; }
            match cache.clear() {
                Ok(()) => status_label.set_text("Cached database cleared"),
                Err(e) => status_label.set_text(&format!("Failed to clear cache: {}", e)),
            }
        });
        dialog.present(Some(&window));
    }));
    options_grid.append(&del_cache_btn);

    {
        let rot_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        rot_row.set_margin_top(4);
        rot_row.set_margin_bottom(4);
        let rot_label = gtk4::Label::new(Some("Rotation-invariant matching"));
        rot_label.set_hexpand(true);
        rot_label.set_halign(gtk4::Align::Start);
        rot_label.set_tooltip_text(Some("Detect images that are rotated 90\u{b0}, 180\u{b0}, or 270\u{b0} copies of each other. Slightly slower scan."));
        let rot_switch = gtk4::CheckButton::new();
        rot_switch.set_active(rotation_enabled.load(Ordering::Relaxed));
        rot_switch.connect_toggled(clone!(#[strong] rotation_enabled, #[strong] auto_save, #[strong] threshold_val,
            #[strong] dirs, #[strong] ref_dirs, move |cb| {
            let val = cb.is_active();
            rotation_enabled.store(val, Ordering::Relaxed);
            save_settings(&auto_save, &rotation_enabled, &threshold_val,
                &*dirs.lock().unwrap(), &*ref_dirs.lock().unwrap());
        }));
        rot_row.append(&rot_label);
        rot_row.append(&rot_switch);
        options_grid.append(&rot_row);
    }

    {
        let thr_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        thr_row.set_margin_top(4);
        thr_row.set_margin_bottom(4);
        let thr_label = gtk4::Label::new(Some("Sensitivity (Hamming threshold, default 2)"));
        thr_label.set_hexpand(true);
        thr_label.set_halign(gtk4::Align::Start);
        thr_label.set_tooltip_text(Some("Lower = stricter matching. 2 = exact/near-exact duplicates only. 8 = similar images allowed. Default: 2."));
        let thr_spin = gtk4::SpinButton::with_range(1.0, 20.0, 1.0);
        thr_spin.set_value(threshold_val.load(Ordering::Relaxed) as f64);
        thr_spin.connect_value_changed(clone!(#[strong] threshold_val, #[strong] auto_save, #[strong] rotation_enabled,
            #[strong] dirs, #[strong] ref_dirs, move |sb| {
            let v = sb.value() as u32;
            threshold_val.store(v, Ordering::Relaxed);
            save_settings(&auto_save, &rotation_enabled, &threshold_val,
                &*dirs.lock().unwrap(), &*ref_dirs.lock().unwrap());
        }));
        thr_row.append(&thr_label);
        thr_row.append(&thr_spin);
        options_grid.append(&thr_row);
        let thr_desc = gtk4::Label::new(Some("Lower = stricter. 2 finds only near-identical images (default). 8 catches similar images too."));
        thr_desc.set_css_classes(&["dim-label"]);
        thr_desc.set_margin_start(12);
        thr_desc.set_margin_bottom(6);
        thr_desc.set_wrap(true);
        thr_desc.set_xalign(0.0);
        options_grid.append(&thr_desc);
    }

    {
        let as_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        as_row.set_margin_top(4);
        as_row.set_margin_bottom(4);
        let as_label = gtk4::Label::new(Some("Auto-save settings"));
        as_label.set_hexpand(true);
        as_label.set_halign(gtk4::Align::Start);
        as_label.set_tooltip_text(Some("When enabled, rotation toggle state is remembered across restarts."));
        let as_switch = gtk4::CheckButton::new();
        as_switch.set_active(auto_save.load(Ordering::Relaxed));
        as_switch.connect_toggled(clone!(#[strong] auto_save, #[strong] rotation_enabled, #[strong] threshold_val,
            #[strong] dirs, #[strong] ref_dirs, move |cb| {
            let val = cb.is_active();
            auto_save.store(val, Ordering::Relaxed);
            save_settings(&auto_save, &rotation_enabled, &threshold_val,
                &*dirs.lock().unwrap(), &*ref_dirs.lock().unwrap());
        }));
        as_row.append(&as_label);
        as_row.append(&as_switch);
        options_grid.append(&as_row);
    }

    let sep = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    sep.set_margin_top(6);
    sep.set_margin_bottom(6);
    options_grid.append(&sep);

    let export_btn = btn_icon_text("Export results", "document-save-as-symbolic");
    export_btn.set_tooltip_text(Some("Save duplicate groups to a JSON file"));
    export_btn.connect_clicked(clone!(#[strong] results_data, #[strong] window,
        #[strong] stats_label, #[strong] options_popover, move |_| {
        options_popover.popdown();
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Export results");
        let rd = results_data.clone();
        let sl = stats_label.clone();
        dialog.save(Some(&window), None::<&gtk4::gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    let data = rd.lock().unwrap();
                    let groups: Vec<dedupe::DuplicateGroup> = data.iter().map(|gd| {
                        let files: Vec<dedupe::ImageEntry> = gd.files.iter().map(|fd| dedupe::ImageEntry {
                            path: std::path::PathBuf::from(&fd.path),
                            size: fd.raw_size,
                            hash: 0,
                            rot_hashes: None,
                            low_variance: false,
                        }).collect();
                        dedupe::DuplicateGroup {
                            hash: 0,
                            files,
                            is_rotation: false,
                        }
                    }).collect();
                    drop(data);
                    let json = serde_json::to_string_pretty(&groups);
                    if let Ok(content) = json {
                        if std::fs::write(&path, &content).is_ok() {
                            sl.set_text(&format!("Results exported to {}", path.display()));
                        }
                    }
                }
            }
        });
    }));
    options_grid.append(&export_btn);

    let import_btn = btn_icon_text("Import results", "document-open-symbolic");
    import_btn.set_tooltip_text(Some("Load duplicate groups from a JSON file"));
    import_btn.connect_clicked(clone!(#[strong] window, #[strong] results_box, #[strong] results_data,
        #[strong] selection, #[strong] no_results_label, #[strong] scrolled,
        #[strong] toolbar_revealer, #[strong] move_sel_btn, #[strong] trash_sel_btn,
        #[strong] stats_label, #[strong] status_label, #[strong] progress_bar,
        #[strong] ref_dirs, #[strong] per_file_refs, #[strong] options_popover, move |_| {
        options_popover.popdown();
        let dialog = gtk4::FileDialog::new();
        dialog.set_title("Import results");
        let sl = stats_label.clone();
        let rbox = results_box.clone();
        let rd = results_data.clone();
        let sel = selection.clone();
        let nol = no_results_label.clone();
        let scr = scrolled.clone();
        let tr = toolbar_revealer.clone();
        let mb = move_sel_btn.clone();
        let tb = trash_sel_btn.clone();
        let stl = stats_label.clone();
        let sul = status_label.clone();
        let pb = progress_bar.clone();
        let rd2 = ref_dirs.clone();
        let pfr = per_file_refs.clone();
        let w = window.clone();
        dialog.open(Some(&window), None::<&gtk4::gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    match std::fs::read_to_string(&path) {
                        Ok(content) => {
                            match serde_json::from_str::<Vec<dedupe::DuplicateGroup>>(&content) {
                                Ok(groups) => {
                                    while let Some(child) = rbox.first_child() {
                                        rbox.remove(&child);
                                    }
                                    rd.lock().unwrap().clear();
                                    let dummy_progress = Arc::new(ProgressState::new());
                                    let refs = rd2.lock().unwrap();
                                    build_results(&rbox, &rd, &sel, &groups, false,
                                        &dummy_progress, &stl, &sul, &pb, &tr, &mb, &tb,
                                        &nol, &scr, &refs, &pfr, &w);
                                    stl.set_text(&format!("Imported {} groups", groups.len()));
                                }
                                Err(e) => {
                                    sl.set_text(&format!("Failed to parse: {}", e));
                                }
                            }
                        }
                        Err(e) => {
                            sl.set_text(&format!("Failed to read: {}", e));
                        }
                    }
                }
            }
        });
    }));
    options_grid.append(&import_btn);

    let about_btn = btn_icon_text("About", "help-about-symbolic");
    about_btn.set_tooltip_text(Some("About ImpHash"));
    about_btn.connect_clicked(clone!(#[strong] window, #[strong] options_popover, move |_| {
        options_popover.popdown();
        show_about_window(window.upcast_ref::<gtk4::Window>());
    }));
    options_grid.append(&about_btn);

    options_popover.set_child(Some(&options_grid));
    options_btn.set_popover(Some(&options_popover));

    // --- Clear search results handler ---
    {
        let dirs_to_clear = dirs.clone();
        let ref_dirs_to_clear = ref_dirs.clone();
        let results_data_to_clear = results_data.clone();
        let selection_to_clear = selection.clone();
        let results_box_to_clear = results_box.clone();
        let no_results_label_to_clear = no_results_label.clone();
        let scrolled_to_clear = scrolled.clone();
        let toolbar_to_clear = toolbar_revealer.clone();
        let scan_btn_to_clear = scan_btn.clone();
        let dir_list_to_clear = dir_list.clone();
        let status_to_clear = status_label.clone();
        let move_btn_to_clear = move_sel_btn.clone();
        let trash_btn_to_clear = trash_sel_btn.clone();
        let select_by_to_clear = select_by_btn.clone();

        clear_results_btn.connect_clicked(clone!(#[strong] window, move |_| {
            let dialog = libadwaita::AlertDialog::builder()
                .heading("Clear Everything?")
                .body("All directories, reference paths, and scan results will be removed.")
                .build();
            dialog.add_response("cancel", "Cancel");
            dialog.add_response("clear", "Clear");
            dialog.set_response_appearance("clear", libadwaita::ResponseAppearance::Destructive);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");

            let dirs = dirs_to_clear.clone();
            let ref_dirs = ref_dirs_to_clear.clone();
            let results_data = results_data_to_clear.clone();
            let selection = selection_to_clear.clone();
            let results_box = results_box_to_clear.clone();
            let no_results_label = no_results_label_to_clear.clone();
            let scrolled = scrolled_to_clear.clone();
            let toolbar = toolbar_to_clear.clone();
            let scan_btn = scan_btn_to_clear.clone();
            let dir_list = dir_list_to_clear.clone();
            let status = status_to_clear.clone();
            let move_btn = move_btn_to_clear.clone();
            let trash_btn = trash_btn_to_clear.clone();
            let select_by = select_by_to_clear.clone();
            dialog.connect_response(None, move |_, response| {
                if response != "clear" { return; }

                dirs.lock().unwrap().clear();
                ref_dirs.lock().unwrap().clear();
                results_data.lock().unwrap().clear();
                selection.lock().unwrap().clear();

                while let Some(child) = results_box.first_child() {
                    results_box.remove(&child);
                }
                while let Some(row) = dir_list.row_at_index(0) {
                    dir_list.remove(&row);
                }
                dir_list.set_visible(false);

                scrolled.set_visible(false);
                no_results_label.set_visible(false);
                toolbar.set_reveal_child(false);
                scan_btn.set_sensitive(false);
                move_btn.set_sensitive(false);
                trash_btn.set_sensitive(false);
                select_by.set_visible(true);
                status.set_text("Cleared all search results and directories");
            });
            dialog.present(Some(&window));
        }));
    }

    // --- Scan handler ---
        scan_btn.connect_clicked(clone!(#[strong] dirs, #[strong] ref_dirs, #[strong] cancel_flag, 
        #[strong] pause_flag, #[strong] cache,
        #[strong] per_file_refs,
        #[strong] results_box, #[strong] no_results_label,
        #[strong] scrolled, #[strong] progress_bar, #[strong] status_label, #[strong] toolbar_revealer,
        #[strong] scan_btn, #[strong] cancel_btn, #[strong] pause_btn, #[strong] stats_label,
        #[strong] move_sel_btn, #[strong] trash_sel_btn, #[strong] results_data, #[strong] selection,
        #[strong] window, move |_| {
        selection.lock().unwrap().clear();
        cancel_flag.store(false, Ordering::Relaxed);
        pause_flag.store(false, Ordering::Relaxed);
        pause_btn.set_label("Pause");

        let dirs_snapshot = dirs.lock().unwrap().clone();
        let dir_paths: Vec<PathBuf> = dirs_snapshot.iter().map(PathBuf::from).collect();
        if dir_paths.is_empty() {
            let dialog = gtk4::AlertDialog::builder()
                .message("No directories selected")
                .detail("Please add at least one directory to scan using the \"Add Directory\" button.")
                .buttons(["OK"])
                .build();
            dialog.choose(Some(&window), None::<&gtk4::gio::Cancellable>, |_| {});
            return;
        }

        scan_btn.set_sensitive(false);
        cancel_btn.set_sensitive(true);
        pause_btn.set_sensitive(true);
        progress_bar.set_fraction(0.0);
        progress_bar.set_show_text(true);
        progress_bar.set_text(Some("Starting..."));
        status_label.set_text("Scanning directories...");
        move_sel_btn.set_sensitive(false);
        trash_sel_btn.set_sensitive(false);
        toolbar_revealer.set_reveal_child(false);
        scrolled.set_visible(false);
        no_results_label.set_visible(false);

        while let Some(child) = results_box.first_child() {
            results_box.remove(&child);
        }
        results_data.lock().unwrap().clear();

        let progress = Arc::new(ProgressState::new());

        // Thread-to-main-thread message channel
        enum ScanMsg {
            ScanError(String),
            NoImages,
            Found(usize),
            Groups(Vec<DuplicateGroup>, bool),
            Error(String),
        }
        let (msg_tx, msg_rx) = std::sync::mpsc::channel::<ScanMsg>();

        // Timer: checks progress + processes scan messages
        // Clone strong refs inside the outer closure (after #[weak] upgrade)
        // so the timer closure holds strong refs, not Downgraded wrappers
        let timer_results_box = results_box.clone();
        let timer_no_results = no_results_label.clone();
        let timer_scrolled = scrolled.clone();
        let timer_toolbar = toolbar_revealer.clone();
        let timer_move_btn = move_sel_btn.clone();
        let timer_trash_btn = trash_sel_btn.clone();
        let timer_cancel_btn = cancel_btn.clone();
        let timer_scan_btn = scan_btn.clone();
        let timer_pause_btn = pause_btn.clone();
        let timer_stats = stats_label.clone();
        let timer_progress = progress.clone();
        let timer_status = status_label.clone();
        let timer_bar = progress_bar.clone();
        let rd = results_data.clone();
        let sel = selection.clone();
        let ref_snapshot = ref_dirs.lock().unwrap().clone();
        let timer_per_file = per_file_refs.clone();
        let timer_window = window.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
            while let Ok(msg) = msg_rx.try_recv() {
                match msg {
                    ScanMsg::ScanError(e) => {
                        timer_status.set_text(&format!("Error: {}", e));
                        timer_cancel_btn.set_sensitive(false);
                        timer_pause_btn.set_sensitive(false);
                        timer_scan_btn.set_sensitive(true);
                    }
                    ScanMsg::NoImages => {
                        timer_status.set_text("No images found");
                        timer_bar.set_fraction(1.0);
                        timer_bar.set_text(Some("Complete - 100%"));
                        timer_cancel_btn.set_sensitive(false);
                        timer_pause_btn.set_sensitive(false);
                        timer_scan_btn.set_sensitive(true);
                        timer_no_results.set_visible(true);
                    }
                    ScanMsg::Found(n) => {
                        timer_status.set_text(&format!("Found {} images. Computing hashes...", n));
                    }
                    ScanMsg::Groups(groups, was_cancelled) => {
                        build_results(&timer_results_box, &rd, &sel, &groups,
                            was_cancelled, &timer_progress, &timer_stats, &timer_status,
                            &timer_bar, &timer_toolbar, &timer_move_btn, &timer_trash_btn,
                            &timer_no_results, &timer_scrolled, &ref_snapshot, &timer_per_file,
                            &timer_window);
                        timer_cancel_btn.set_sensitive(false);
                        timer_pause_btn.set_sensitive(false);
                        timer_scan_btn.set_sensitive(true);
                        timer_bar.set_fraction(1.0);
                        timer_bar.set_text(Some("Complete - 100%"));
                        return glib::ControlFlow::Break;
                    }
                    ScanMsg::Error(e) => {
                        timer_status.set_text(&format!("Error: {}", e));
                        timer_cancel_btn.set_sensitive(false);
                        timer_pause_btn.set_sensitive(false);
                        timer_scan_btn.set_sensitive(true);
                        return glib::ControlFlow::Break;
                    }
                }
            }

            if !timer_progress.dirty.swap(false, Ordering::Relaxed) {
                return glib::ControlFlow::Continue;
            }
            let done = timer_progress.done.load(Ordering::Relaxed);
            let total = timer_progress.total.load(Ordering::Relaxed);
            let failed = timer_progress.failed.load(Ordering::Relaxed);
            let is_hashing = timer_progress.is_hashing.load(Ordering::Relaxed);
            let (frac, msg) = if is_hashing {
                let p = (done as f64 / total.max(1) as f64) * 0.85 + 0.05;
                let percent = ((done as f64 / total.max(1) as f64) * 85.0 + 5.0) as i32;
                let m = if failed > 0 {
                    format!("Hashing: {}/{} ({} failed) - {}%", done, total, failed, percent)
                } else {
                    format!("Hashing: {}/{} - {}%", done, total, percent)
                };
                (p, m)
            } else {
                let p = ((done as f64 / total.max(1) as f64) * 0.10) + 0.90;
                let percent = ((done as f64 / total.max(1) as f64) * 10.0 + 90.0) as i32;
                let m = if failed > 0 {
                    format!("Comparing: {}/{} ({} skipped) - {}%", done, total, failed, percent)
                } else {
                    format!("Comparing: {}/{} - {}%", done, total, percent)
                };
                (p, m)
            };
            timer_bar.set_fraction(frac as f64);
            timer_bar.set_text(Some(&msg));
            timer_status.set_text(&msg);
            glib::ControlFlow::Continue
        });

        // Background thread: only Send types
        let tx = msg_tx.clone();
        let cancel = cancel_flag.clone();
        let pause = pause_flag.clone();
        let cache = cache.clone();
        let progress = progress.clone();
        let rot_flag = rotation_enabled.clone();
        let thr_flag = threshold_val.clone();
        std::thread::spawn(move || {
            let paths = match scanner::find_images_multi(&dir_paths) {
                Ok(p) => p,
                Err(e) => {
                    let _ = tx.send(ScanMsg::ScanError(e.to_string()));
                    return;
                }
            };
            if cancel.load(Ordering::Relaxed) { return; }
            if paths.is_empty() {
                let _ = tx.send(ScanMsg::NoImages);
                return;
            }
            let n = paths.len();
            let _ = tx.send(ScanMsg::Found(n));

            let rotation_mode = rot_flag.load(Ordering::Relaxed);
            let thr = thr_flag.load(Ordering::Relaxed);
            let groups = dedupe::find_duplicates(
                &paths, &cancel, Some(&*cache), rotation_mode, thr,
                |done, total, failed, cur_file| {
                    // Check for pause and wait if paused
                    while pause.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                    let is_hash = !cur_file.is_empty();
                    progress.done.store(done, Ordering::Relaxed);
                    progress.total.store(total, Ordering::Relaxed);
                    progress.failed.store(failed, Ordering::Relaxed);
                    progress.is_hashing.store(is_hash, Ordering::Relaxed);
                    progress.dirty.store(true, Ordering::Release);
                },
            );
            let was_cancelled = cancel.load(Ordering::Relaxed);
            let _ = cache.save();
            match groups {
                Ok(groups) => { let _ = tx.send(ScanMsg::Groups(groups, was_cancelled)); }
                Err(e) => { let _ = tx.send(ScanMsg::Error(e.to_string())); }
            }
        });
    }));

    window.present();
}

struct FileData {
    path: String,
    raw_size: u64,
    check: gtk4::CheckButton,
    name_label: gtk4::Label,
    row: gtk4::Box,
    reference: bool,
    ref_badge: gtk4::Label,
    deleted_label: gtk4::Label,
    moved_label: gtk4::Label,
    trash_btn: gtk4::Button,
    move_btn: gtk4::Button,
    restore_btn: gtk4::Button,
}

struct GroupData {
    expander: gtk4::Expander,
    files: Vec<FileData>,
}

fn set_ref_styling(fd: &mut FileData, is_ref: bool) {
    fd.reference = is_ref;
    fd.ref_badge.set_visible(is_ref);
    if is_ref {
        fd.row.set_css_classes(&["ref-row"]);
        fd.name_label.set_css_classes(&["ref-path"]);
    } else {
        fd.row.set_css_classes(&[]);
        fd.name_label.set_css_classes(&[]);
    }
}

fn refresh_all_ref_styling(
    results_data: &Arc<Mutex<Vec<GroupData>>>,
    ref_dirs: &Arc<Mutex<HashSet<String>>>,
) {
    let mut data = results_data.lock().unwrap();
    for gd in data.iter_mut() {
        for fd in gd.files.iter_mut() {
            let is_ref = is_ref_path(&fd.path, ref_dirs);
            set_ref_styling(fd, is_ref);
        }
    }
}

fn is_ref_path(path: &str, ref_dirs: &Mutex<HashSet<String>>) -> bool {
    let rd = ref_dirs.lock().unwrap();
    rd.iter().any(|d| {
        if !path.starts_with(d) { return false; }
        let rem = &path[d.len()..];
        rem.is_empty() || rem.starts_with('/')
    })
}

fn do_select_all(
    results_data: &Arc<Mutex<Vec<GroupData>>>,
    selection: &Arc<Mutex<HashSet<String>>>,
    ref_dirs: &Arc<Mutex<HashSet<String>>>,
    move_sel_btn: &gtk4::Button,
    trash_sel_btn: &gtk4::Button,
    stats_label: &gtk4::Label,
) {
    let data = results_data.lock().unwrap();
    let to_select: Vec<String> = data.iter()
        .flat_map(|gd| gd.files.iter())
        .filter(|fd| !is_ref_path(&fd.path, ref_dirs))
        .map(|fd| fd.path.clone())
        .collect();
    let n = to_select.len();

    {
        let mut sel = selection.lock().unwrap();
        sel.clear();
        for p in &to_select {
            sel.insert(p.clone());
        }
    }

    for gd in data.iter() {
        for fd in gd.files.iter() {
            if !is_ref_path(&fd.path, ref_dirs) && !fd.check.is_active() {
                fd.check.set_active(true);
            }
        }
        gd.expander.set_expanded(true);
    }
    drop(data);

    move_sel_btn.set_sensitive(n > 0);
    trash_sel_btn.set_sensitive(n > 0);
    stats_label.set_text(&format!("Selected: {}", n));
}

fn read_exif_date(path: &str) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    // Only attempt JPEG (SOI marker FF D8)
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    // Scan APP1 marker (FF E1) which contains EXIF
    let mut i = 2usize;
    while i + 3 < data.len() {
        if data[i] != 0xFF { break; }
        let marker = data[i + 1];
        let seg_len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        if marker == 0xE1 && i + 4 + seg_len <= data.len() {
            let seg = &data[i + 4..i + 2 + seg_len];
            // Check for "Exif\0\0" header
            if seg.len() > 6 && &seg[0..6] == b"Exif\0\0" {
                let tiff = &seg[6..];
                if tiff.len() < 8 { return None; }
                let little_endian = &tiff[0..2] == b"II";
                let read_u16 = |buf: &[u8], off: usize| -> Option<u16> {
                    let b = buf.get(off..off+2)?;
                    Some(if little_endian { u16::from_le_bytes([b[0], b[1]]) }
                         else { u16::from_be_bytes([b[0], b[1]]) })
                };
                let read_u32 = |buf: &[u8], off: usize| -> Option<u32> {
                    let b = buf.get(off..off+4)?;
                    Some(if little_endian { u32::from_le_bytes([b[0], b[1], b[2], b[3]]) }
                         else { u32::from_be_bytes([b[0], b[1], b[2], b[3]]) })
                };
                let ifd0_offset = read_u32(tiff, 4)? as usize;
                let ifd0_count = read_u16(tiff, ifd0_offset)? as usize;
                // Look for ExifIFD pointer (tag 0x8769)
                let mut exif_ifd_offset: Option<usize> = None;
                for t in 0..ifd0_count {
                    let entry = ifd0_offset + 2 + t * 12;
                    if entry + 12 > tiff.len() { break; }
                    let tag = read_u16(tiff, entry)?;
                    if tag == 0x8769 {
                        exif_ifd_offset = Some(read_u32(tiff, entry + 8)? as usize);
                        break;
                    }
                }
                // Search both IFD0 and ExifIFD for DateTimeOriginal (0x9003) or DateTime (0x0132)
                let ifds: Vec<usize> = std::iter::once(ifd0_offset)
                    .chain(exif_ifd_offset)
                    .collect();
                for ifd_off in ifds {
                    let count = match read_u16(tiff, ifd_off) { Some(c) => c as usize, None => continue };
                    for t in 0..count {
                        let entry = ifd_off + 2 + t * 12;
                        if entry + 12 > tiff.len() { break; }
                        let tag = match read_u16(tiff, entry) { Some(v) => v, None => break };
                        if tag == 0x9003 || tag == 0x0132 {
                            let offset = read_u32(tiff, entry + 8)? as usize;
                            if offset + 19 <= tiff.len() {
                                let s = std::str::from_utf8(&tiff[offset..offset+19]).ok()?;
                                // EXIF date: "YYYY:MM:DD HH:MM:SS" → "YYYY-MM-DD HH:MM"
                                if s.len() >= 16 {
                                    let parts: Vec<&str> = s.splitn(3, ' ').collect();
                                    if parts.len() == 2 {
                                        let date = parts[0].replace(':', "-");
                                        let time = &parts[1][..5]; // HH:MM
                                        return Some(format!("📅 {} {}", date, time));
                                    }
                                    // Fallback: replace all colons
                                    let d = s.replace(':', "-");
                                    return Some(format!("📅 {}", &d[..16]));
                                }
                            }
                        }
                    }
                }
            }
        }
        if seg_len < 2 { break; }
        i += 2 + seg_len;
    }
    None
}

fn build_results(results_box: &gtk4::Box,
    results_data: &Arc<Mutex<Vec<GroupData>>>,
    selection: &Arc<Mutex<HashSet<String>>>,
    groups: &[DuplicateGroup],
    was_cancelled: bool,
    progress: &Arc<ProgressState>,
    stats_label: &gtk4::Label,
    status_label: &gtk4::Label,
    progress_bar: &gtk4::ProgressBar,
    toolbar_revealer: &gtk4::Revealer,
    move_sel_btn: &gtk4::Button,
    trash_sel_btn: &gtk4::Button,
    no_results_label: &gtk4::Label,
    scrolled: &gtk4::ScrolledWindow,
    ref_dirs: &HashSet<String>,
    per_file_refs: &Arc<Mutex<HashSet<String>>>,
    window: &impl IsA<gtk4::Window>,
) {
    let pfr_lock = per_file_refs.lock().unwrap();
    let pfr_snapshot = pfr_lock.clone();
    drop(pfr_lock);
    let total_duplicates: usize = groups.iter().map(|g| g.files.len()).sum();
    let total_groups = groups.len();

    let mut new_data = Vec::new();

    for (_gi, group) in groups.iter().enumerate() {
        let group_label = if group.is_rotation {
            format!("Duplicate Group #{:016x}  ({} files)  🔄 rotation-matched", group.hash, group.files.len())
        } else {
            format!("Duplicate Group #{:016x}  ({} files)", group.hash, group.files.len())
        };
        let expander = gtk4::Expander::new(Some(&group_label));
        expander.set_expanded(true);
        let files_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        files_box.set_margin_start(20);
        let col_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        col_header.set_css_classes(&["col-header-row"]);
        col_header.set_margin_bottom(2);
        col_header.set_margin_top(2);
        let ch_check = gtk4::Label::new(None);
        ch_check.set_width_chars(2);
        let ch_name = gtk4::Label::new(Some("Path"));
        ch_name.set_css_classes(&["column-header"]);
        ch_name.set_hexpand(true);
        ch_name.set_halign(gtk4::Align::Start);
        let mk_col_sep = || {
            let s = gtk4::Separator::new(gtk4::Orientation::Vertical);
            s.set_margin_top(2);
            s.set_margin_bottom(2);
            s
        };
        let ch_date = gtk4::Label::new(Some("Date Taken"));
        ch_date.set_css_classes(&["column-header"]);
        ch_date.set_size_request(150, -1);
        ch_date.set_halign(gtk4::Align::Start);
        let ch_res = gtk4::Label::new(Some("Resolution"));
        ch_res.set_css_classes(&["column-header"]);
        ch_res.set_size_request(100, -1);
        ch_res.set_halign(gtk4::Align::Start);
        let ch_size = gtk4::Label::new(Some("Size"));
        ch_size.set_css_classes(&["column-header"]);
        ch_size.set_size_request(80, -1);
        ch_size.set_halign(gtk4::Align::Start);
        let ch_actions = gtk4::Label::new(Some("Actions"));
        ch_actions.set_css_classes(&["column-header"]);
        ch_actions.set_size_request(170, -1);
        ch_actions.set_halign(gtk4::Align::End);
        // Right-side header columns in a fixed box so separators always align
        let ch_right = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        ch_right.set_halign(gtk4::Align::End);
        ch_right.append(&mk_col_sep());
        ch_right.append(&ch_date);
        ch_right.append(&mk_col_sep());
        ch_right.append(&ch_res);
        ch_right.append(&mk_col_sep());
        ch_right.append(&ch_size);
        ch_right.append(&mk_col_sep());
        ch_right.append(&ch_actions);
        col_header.append(&ch_check);
        col_header.append(&ch_name);
        col_header.append(&ch_right);
        files_box.append(&col_header);

        let mut file_datas = Vec::new();
        for (_fi, entry) in group.files.iter().enumerate() {
            let path_str = entry.path.to_string_lossy().to_string();
            let dir_ref = ref_dirs.iter().any(|d| path_str.starts_with(d) && {
                let rem = &path_str[d.len()..];
                rem.is_empty() || rem.starts_with('/')
            });
            let is_ref = dir_ref || pfr_snapshot.contains(&path_str);
            let size_str = preview::format_size(entry.size);
            let res_str = image::image_dimensions(std::path::Path::new(&path_str))
                .ok().map(|(w, h)| format!("{}x{}", w, h)).unwrap_or_default();
            let date_str = read_exif_date(&path_str).unwrap_or_default();
            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
            row.set_css_classes(&["result-row"]);
            row.set_margin_top(2);
            row.set_margin_bottom(2);

            let check = gtk4::CheckButton::new();
            // checkboxes are always enabled so the user can manually select refs
            let name_label = gtk4::Label::new(Some(&path_str));
            name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
            name_label.set_hexpand(true);
            name_label.set_halign(gtk4::Align::Start);
            let res_label = gtk4::Label::new(Some(&res_str));
            res_label.set_css_classes(&["dim-label"]);
            res_label.set_size_request(100, -1);
            res_label.set_halign(gtk4::Align::Start);
            let size_label = gtk4::Label::new(Some(&size_str));
            size_label.set_css_classes(&["dim-label"]);
            size_label.set_size_request(80, -1);
            size_label.set_halign(gtk4::Align::Start);
            let date_label = gtk4::Label::new(Some(&date_str));
            date_label.set_css_classes(&["dim-label"]);
            date_label.set_size_request(150, -1);
            date_label.set_halign(gtk4::Align::Start);
            let move_btn = btn_icon_text("Move", "go-jump-symbolic");
            move_btn.set_css_classes(&["small"]);
            let trash_btn = btn_icon_text("Trash", "user-trash-symbolic");
            trash_btn.set_css_classes(&["small", "destructive-action"]);
            let restore_btn = btn_icon_text("Restore", "document-revert-symbolic");
            restore_btn.set_css_classes(&["small"]);
            restore_btn.set_visible(false);
            let deleted_label = gtk4::Label::new(Some("🗑 Moved to trash"));
            deleted_label.set_css_classes(&["error"]);
            deleted_label.set_visible(false);
            deleted_label.set_margin_start(12);
            deleted_label.set_margin_end(12);
            let moved_label = gtk4::Label::new(Some(""));
            moved_label.set_css_classes(&["dim-label"]);
            moved_label.set_visible(false);
            moved_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            moved_label.set_margin_start(12);
            moved_label.set_margin_end(12);
            if is_ref {
                row.set_css_classes(&["ref-row"]);
                name_label.set_css_classes(&["ref-path"]);
            }

            let ref_badge = gtk4::Label::new(Some("REF"));
            ref_badge.set_css_classes(&["status-pill-ref"]);
            ref_badge.set_visible(is_ref);

            let mk_row_sep = || {
                let s = gtk4::Separator::new(gtk4::Orientation::Vertical);
                s.set_margin_top(4);
                s.set_margin_bottom(4);
                s
            };
            // Right-side data columns mirror the header box exactly
            let row_right = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
            row_right.set_halign(gtk4::Align::End);
            let actions_cell = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
            actions_cell.set_size_request(170, -1);
            actions_cell.set_halign(gtk4::Align::End);
            actions_cell.append(&move_btn);
            actions_cell.append(&trash_btn);
            actions_cell.append(&restore_btn);
            row_right.append(&mk_row_sep());
            row_right.append(&date_label);
            row_right.append(&mk_row_sep());
            row_right.append(&res_label);
            row_right.append(&mk_row_sep());
            row_right.append(&size_label);
            row_right.append(&mk_row_sep());
            row_right.append(&actions_cell);
            row.append(&check);
            row.append(&name_label);
            row.append(&ref_badge);
            row.append(&deleted_label);
            row.append(&moved_label);
            row.append(&row_right);

            let gesture = gtk4::GestureClick::new();
            gesture.set_button(1);
            gesture.connect_pressed(clone!(#[strong] path_str, move |_, n_press, _, _| {
                if n_press == 2 {
                    let _ = std::process::Command::new("xdg-open")
                        .arg(&path_str)
                        .spawn();
                }
            }));
            row.add_controller(gesture);

            check.connect_toggled(clone!(#[strong] selection, #[strong] path_str,
                #[weak] stats_label, #[weak] move_sel_btn, #[weak] trash_sel_btn, move |cb| {
                let mut sel = selection.lock().unwrap();
                if cb.is_active() {
                    sel.insert(path_str.clone());
                } else {
                    sel.remove(path_str.as_str());
                }
                let n = sel.len();
                let has_sel = !sel.is_empty();
                stats_label.set_text(&format!("Selected: {}", n));
                move_sel_btn.set_sensitive(has_sel);
                trash_sel_btn.set_sensitive(has_sel);
            }));

            let pw = window.clone();
            let check_clone = check.clone();
            let check_clone2 = check.clone();
            move_btn.connect_clicked(clone!(#[strong] path_str, #[strong] status_label,
                #[strong] moved_label, #[strong] move_btn, move |_| {
                if !check_clone.is_active() {
                    status_label.set_text("Select the file to move");
                    status_label.set_css_classes(&["error"]);
                    let sl = status_label.clone();
                    glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
                        sl.set_css_classes(&[]);
                        sl.set_text("");
                        glib::ControlFlow::Break
                    });
                    return;
                }
                move_single_file_with_label(&path_str, &status_label, &pw, &moved_label, &move_btn);
            }));
            trash_btn.connect_clicked(clone!(#[strong] path_str, #[strong] status_label,
                #[strong] deleted_label, #[strong] row, #[strong] trash_btn, #[strong] restore_btn, move |_| {
                if !check_clone2.is_active() {
                    status_label.set_text("Select the file to trash");
                    status_label.set_css_classes(&["error"]);
                    let sl = status_label.clone();
                    glib::timeout_add_local(std::time::Duration::from_millis(1500), move || {
                        sl.set_css_classes(&[]);
                        sl.set_text("");
                        glib::ControlFlow::Break
                    });
                    return;
                }
                match trash::delete(&path_str) {
                    Ok(()) => {
                        status_label.set_text(&format!("Trashed: {}", path_str));
                        deleted_label.set_visible(true);
                        trash_btn.set_visible(false);
                        restore_btn.set_visible(true);
                        row.set_css_classes(&["result-row", "deleted"]);
                    }
                    Err(e) => {
                        status_label.set_text(&format!("Failed to trash: {}", e));
                    }
                }
            }));

            let sl_for_restore = status_label.clone();
            let restored_row = row.clone();
            let restored_deleted = deleted_label.clone();
            let restored_trash = trash_btn.clone();
            let restored_restore = restore_btn.clone();
            let rp = path_str.clone();
            let was_ref = is_ref;
            restore_btn.connect_clicked(move |_| {
                let items = match trash::os_limited::list() {
                    Ok(items) => items,
                    Err(e) => {
                        sl_for_restore.set_text(&format!("Failed to list trash: {}", e));
                        return;
                    }
                };
                let to_restore: Vec<trash::TrashItem> = items.into_iter()
                    .filter(|item| item.original_path() == std::path::Path::new(&rp))
                    .collect();
                if to_restore.is_empty() {
                    sl_for_restore.set_text("File not found in trash");
                    return;
                }
                match trash::os_limited::restore_all(to_restore) {
                    Ok(()) => {
                        sl_for_restore.set_text(&format!("Restored: {}", rp));
                        restored_deleted.set_visible(false);
                        restored_trash.set_visible(true);
                        restored_restore.set_visible(false);
                        if was_ref {
                            restored_row.set_css_classes(&["result-row", "ref-row"]);
                        } else {
                            restored_row.set_css_classes(&["result-row"]);
                        }
                    }
                    Err(e) => {
                        sl_for_restore.set_text(&format!("Failed to restore: {}", e));
                    }
                }
            });

            files_box.append(&row);
            file_datas.push(FileData {
                path: path_str,
                raw_size: entry.size,
                check: check.clone(),
                name_label: name_label.clone(),
                row: row.clone(),
                reference: is_ref,
                ref_badge: ref_badge.clone(),
                deleted_label: deleted_label.clone(),
                moved_label: moved_label.clone(),
                trash_btn: trash_btn.clone(),
                move_btn: move_btn.clone(),
                restore_btn: restore_btn.clone(),
            });
        }
        let group_paths: Vec<String> = file_datas.iter().map(|fd| fd.path.clone()).collect();
            let group_view_btn = btn_icon_text("View", "image-x-generic-symbolic");
            group_view_btn.set_css_classes(&["small"]);
        {
            let paths = group_paths;
            let rd = results_data.clone();
            group_view_btn.connect_clicked(move |_| {
                let entries: Vec<(String, bool)> = {
                    let data = rd.lock().unwrap();
                    data.iter()
                        .flat_map(|gd| gd.files.iter())
                        .filter(|fd| paths.contains(&fd.path))
                        .map(|fd| (fd.path.clone(), fd.reference))
                        .collect()
                };
                show_group_preview(&entries, &rd);
            });
        }
        let group_header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        group_header.set_css_classes(&["group-header"]);
        group_header.append(&group_view_btn);
        files_box.prepend(&group_header);
        expander.set_child(Some(&files_box));
        let frame = gtk4::Frame::new(None);
        frame.set_css_classes(&["group-frame"]);
        frame.set_child(Some(&expander));
        frame.set_margin_bottom(6);
        results_box.append(&frame);
        new_data.push(GroupData { expander, files: file_datas });
    }

    *results_data.lock().unwrap() = new_data;

    let f = progress.failed.load(Ordering::Relaxed);
    let status = if was_cancelled {
        if f > 0 {
            format!("Scan cancelled \u{2014} {} file(s) could not be read", f)
        } else {
            "Scan cancelled \u{2014} partial results".into()
        }
    } else {
        if f > 0 {
            format!("Scan complete \u{2014} {} file(s) could not be read", f)
        } else {
            "Scan complete".into()
        }
    };
    status_label.set_text(&status);
    progress_bar.set_fraction(1.0);
    progress_bar.set_show_text(false);

    if total_groups > 0 {
        scrolled.set_visible(true);
        no_results_label.set_visible(false);
        toolbar_revealer.set_reveal_child(true);
        stats_label.set_text(&format!("Found {} duplicates in {} groups", total_duplicates, total_groups));
        move_sel_btn.set_sensitive(false);
        trash_sel_btn.set_sensitive(false);
    } else {
        scrolled.set_visible(false);
        no_results_label.set_visible(true);
        toolbar_revealer.set_reveal_child(false);
    }
}

fn apply_select_by(data: &[GroupData],
    selection: &Arc<Mutex<HashSet<String>>>,
    mode: i32,
    status_label: &gtk4::Label,
) {
    // Uncheck everything first (without holding selection lock)
    for gd in data {
        for fd in &gd.files {
            fd.check.set_active(false);
        }
    }
    // Clear selection without holding lock during set_active calls
    selection.lock().unwrap().clear();

    if mode != 2 && mode != 3 {
        // Synchronous per-group best selection
        let mut count = 0;
        let mut skipped = 0usize;
        for gd in data {
            let non_ref: Vec<&FileData> = gd.files.iter().filter(|f| !f.reference).collect();
            if non_ref.is_empty() { continue; }

            let all_same = match mode {
                0 | 1 => {
                    let vals: Vec<u64> = non_ref.iter().map(|f| f.raw_size).collect();
                    vals.len() > 1 && vals.iter().min() == vals.iter().max()
                }
                4 | 5 => {
                    let vals: Vec<usize> = non_ref.iter().map(|f| f.path.len()).collect();
                    vals.len() > 1 && vals.iter().min() == vals.iter().max()
                }
                _ => false,
            };
            if all_same {
                skipped += 1;
                continue;
            }

            // set_active triggers toggled handler which locks selection — fine since we don't hold it
            match mode {
                0 => {
                    if let Some(fd) = non_ref.iter().max_by_key(|f| f.raw_size) {
                        fd.check.set_active(true);
                        count += 1;
                    }
                }
                1 => {
                    if let Some(fd) = non_ref.iter().min_by_key(|f| f.raw_size) {
                        fd.check.set_active(true);
                        count += 1;
                    }
                }
                4 => {
                    if let Some(fd) = non_ref.iter().max_by_key(|f| f.path.len()) {
                        fd.check.set_active(true);
                        count += 1;
                    }
                }
                5 => {
                    if let Some(fd) = non_ref.iter().min_by_key(|f| f.path.len()) {
                        fd.check.set_active(true);
                        count += 1;
                    }
                }
                6 => {
                    let has_ref = gd.files.iter().any(|f| f.reference);
                    if has_ref {
                        for fd in &non_ref {
                            fd.check.set_active(true);
                            count += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        if skipped > 0 {
            status_label.set_text(&format!("Selected {} files ({} groups skipped - all identical)", count, skipped));
        } else {
            status_label.set_text(&format!("Selected {} files by criteria", count));
        }
        return;
    }

    // Resolution modes (2, 3): per-group best via background thread
    status_label.set_text("Checking image dimensions\u{2026}");

    let group_info: Vec<(usize, String)> = data.iter().enumerate()
        .flat_map(|(gi, gd)| {
            gd.files.iter()
                .filter(|f| !f.reference)
                .map(move |f| (gi, f.path.clone()))
        })
        .collect();

    let path_to_check: std::collections::HashMap<String, gtk4::CheckButton> = data.iter()
        .flat_map(|gd| gd.files.iter())
        .filter(|f| !f.reference)
        .map(|f| (f.path.clone(), f.check.clone()))
        .collect();

    let (dim_tx, dim_rx) = std::sync::mpsc::channel::<(HashSet<String>, usize)>();

    let mode_captured = mode;
    std::thread::spawn(move || {
        use std::collections::HashMap;
        let mut path_area: HashMap<String, u64> = HashMap::new();
        for (_gi, path) in &group_info {
            if let Ok((w, h)) = image::image_dimensions(std::path::Path::new(path)) {
                path_area.insert(path.clone(), w as u64 * h as u64);
            }
        }
        let mut by_group: std::collections::HashMap<usize, Vec<String>> = std::collections::HashMap::new();
        for (gi, path) in &group_info {
            by_group.entry(*gi).or_default().push(path.clone());
        }
        let mut skipped = 0usize;
        let matching: HashSet<String> = by_group.into_iter()
            .filter_map(|(_gi, paths)| {
                let areas: Vec<u64> = paths.iter().filter_map(|p| path_area.get(p).copied()).collect();
                if areas.len() > 1 && areas.iter().min() == areas.iter().max() {
                    skipped += 1;
                    return None;
                }
                let best = if mode_captured == 2 {
                    paths.iter().max_by_key(|p| path_area.get(*p).copied().unwrap_or(0))
                } else {
                    paths.iter().min_by_key(|p| path_area.get(*p).copied().unwrap_or(u64::MAX))
                };
                best.cloned()
            })
            .collect();
        let _ = dim_tx.send((matching, skipped));
    });

    let sel = selection.clone();
    let sl = status_label.clone();
    glib::idle_add_local(move || {
        match dim_rx.try_recv() {
            Ok((matching, skipped)) => {
                // Set checkboxes first (triggers toggle handlers that update selection)
                for (path, check) in &path_to_check {
                    check.set_active(matching.contains(path));
                }
                let count = sel.lock().unwrap().len();
                if skipped > 0 {
                    sl.set_text(&format!("Selected {} files ({} groups skipped - all identical)", count, skipped));
                } else {
                    sl.set_text(&format!("Selected {} files by criteria", count));
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

fn load_pixbuf_scaled(path: &str, max_size: i32) -> Option<gdk_pixbuf::Pixbuf> {
    let pixbuf = gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    let w = pixbuf.width();
    let h = pixbuf.height();
    if w > max_size || h > max_size {
        let scale = max_size as f64 / w.max(h) as f64;
        Some(pixbuf.scale_simple(
            (w as f64 * scale) as i32,
            (h as f64 * scale) as i32,
            gdk_pixbuf::InterpType::Bilinear,
        )?)
    } else {
        Some(pixbuf)
    }
}

fn move_single_file_with_label(
    path: &str,
    status_label: &gtk4::Label,
    window: &impl IsA<gtk4::Window>,
    moved_label: &gtk4::Label,
    move_btn: &gtk4::Button,
) {
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Select destination folder");
    let p = path.to_owned();
    let sl = status_label.clone();
    let w = window.clone();
    let ml = moved_label.clone();
    let mb = move_btn.clone();
    dialog.select_folder(Some(&w), None::<&gtk4::gio::Cancellable>, move |result| {
        if let Ok(file) = result {
            if let Some(dest) = file.path() {
                let src = std::path::Path::new(&p);
                let target = dest.join(src.file_name().unwrap_or_default());
                if std::fs::rename(&p, &target).is_ok()
                    || (std::fs::copy(&p, &target).is_ok() && std::fs::remove_file(&p).is_ok())
                {
                    let msg = format!("→ {}", target.display());
                    sl.set_text(&format!("Moved to {}", target.display()));
                    ml.set_text(&msg);
                    ml.set_visible(true);
                    mb.set_visible(false);
                } else {
                    sl.set_text(&format!("Failed to move: {}", p));
                }
            }
        }
    });
}

fn move_single_file_with_callback<F>(
    path: &str,
    status_label: &gtk4::Label,
    window: &impl IsA<gtk4::Window>,
    move_btn: &gtk4::Button,
    on_success: F,
) where F: Fn(String) + 'static {
    let dialog = gtk4::FileDialog::new();
    dialog.set_title("Select destination folder");
    let p = path.to_owned();
    let sl = status_label.clone();
    let w = window.clone();
    let mb = move_btn.clone();
    dialog.select_folder(Some(&w), None::<&gtk4::gio::Cancellable>, move |result| {
        if let Ok(file) = result {
            if let Some(dest) = file.path() {
                let src = std::path::Path::new(&p);
                let target = dest.join(src.file_name().unwrap_or_default());
                if std::fs::rename(&p, &target).is_ok()
                    || (std::fs::copy(&p, &target).is_ok() && std::fs::remove_file(&p).is_ok())
                {
                    let dest_str = target.display().to_string();
                    sl.set_text(&format!("→ Moved to {}", dest_str));
                    mb.set_visible(false);
                    on_success(dest_str);
                } else {
                    sl.set_text(&format!("Failed to move: {}", p));
                }
            }
        }
    });
}

fn show_group_preview(
    entries: &[(String, bool)],
    results_data: &Arc<Mutex<Vec<GroupData>>>,
) {
    let window = gtk4::Window::new();
    window.set_title(Some("Group Preview"));
    window.set_default_size(900, 700);
    window.maximize();

    struct MainWidgets {
        deleted_label: gtk4::Label,
        moved_label: gtk4::Label,
        trash_btn: gtk4::Button,
        move_btn: gtk4::Button,
        restore_btn: gtk4::Button,
        row: gtk4::Box,
        reference: bool,
    }
    let main_widgets: HashMap<String, MainWidgets> = results_data
        .lock()
        .unwrap()
        .iter()
        .flat_map(|gd| gd.files.iter())
        .map(|fd| (fd.path.clone(), MainWidgets {
            deleted_label: fd.deleted_label.clone(),
            moved_label: fd.moved_label.clone(),
            trash_btn: fd.trash_btn.clone(),
            move_btn: fd.move_btn.clone(),
            restore_btn: fd.restore_btn.clone(),
            row: fd.row.clone(),
            reference: fd.reference,
        }))
        .collect();
    let main_widgets = std::rc::Rc::new(main_widgets);

    let status_label = gtk4::Label::new(Some(""));
    status_label.set_halign(gtk4::Align::Start);
    status_label.set_margin_top(4);
    status_label.set_css_classes(&["dim-label"]);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    let preview_viewport = gtk4::Viewport::new(None::<&gtk4::Adjustment>, None::<&gtk4::Adjustment>);

    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    main_box.set_margin_top(12);
    main_box.set_margin_bottom(12);
    main_box.set_margin_start(12);
    main_box.set_margin_end(12);

    let n = entries.len();
    let title = gtk4::Label::new(Some(&format!("Previewing {} image{}", n, if n == 1 { "" } else { "s" })));
    title.set_css_classes(&["heading"]);
    title.set_halign(gtk4::Align::Start);
    title.set_margin_bottom(4);
    main_box.append(&title);

    for (path, is_ref) in entries {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        row.set_css_classes(&["card"]);
        row.set_margin_bottom(4);

        let image_box = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        image_box.set_valign(gtk4::Align::Center);

        let pic_frame = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        pic_frame.set_valign(gtk4::Align::Center);
        pic_frame.set_halign(gtk4::Align::Center);
        pic_frame.set_size_request(200, 160);

        let picture = gtk4::Picture::new();
        picture.set_size_request(200, 160);
        picture.set_halign(gtk4::Align::Fill);
        picture.set_valign(gtk4::Align::Fill);
        picture.set_content_fit(gtk4::ContentFit::Contain);
        if let Some(pixbuf) = load_pixbuf_scaled(path, 1200) {
            picture.set_paintable(Some(&gtk4::gdk::Texture::for_pixbuf(&pixbuf)));
        }

        // Overlay shown when file is trashed or moved
        let status_overlay_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        status_overlay_box.set_valign(gtk4::Align::Center);
        status_overlay_box.set_halign(gtk4::Align::Center);
        status_overlay_box.set_size_request(200, 160);
        status_overlay_box.set_visible(false);
        let status_icon = gtk4::Image::new();
        status_icon.set_pixel_size(48);
        status_icon.set_halign(gtk4::Align::Center);
        status_icon.set_margin_top(20);
        let status_overlay_label = gtk4::Label::new(None);
        status_overlay_label.set_halign(gtk4::Align::Center);
        status_overlay_label.set_css_classes(&["dim-label"]);
        status_overlay_label.set_wrap(true);
        status_overlay_label.set_max_width_chars(22);
        status_overlay_label.set_justify(gtk4::Justification::Center);
        let moved_to_label = gtk4::Label::new(None);
        moved_to_label.set_halign(gtk4::Align::Center);
        moved_to_label.set_css_classes(&["dim-label"]);
        moved_to_label.set_wrap(true);
        moved_to_label.set_max_width_chars(22);
        moved_to_label.set_justify(gtk4::Justification::Center);
        moved_to_label.set_visible(false);
        status_overlay_box.append(&status_icon);
        status_overlay_box.append(&status_overlay_label);
        status_overlay_box.append(&moved_to_label);

        let path_clone2 = path.clone();
        let click = gtk4::GestureClick::new();
        click.set_button(1);
        click.connect_pressed(move |_, _, _, _| {
            let _ = std::process::Command::new("xdg-open").arg(&path_clone2).spawn();
        });
        picture.add_controller(click);

        pic_frame.append(&picture);
        pic_frame.append(&status_overlay_box);
        image_box.append(&pic_frame);

        if *is_ref {
            pic_frame.set_css_classes(&["ref-image"]);
            let ref_label = gtk4::Label::new(Some("Reference"));
            ref_label.set_css_classes(&["ref-path"]);
            ref_label.set_halign(gtk4::Align::Center);
            ref_label.set_margin_top(2);
            image_box.append(&ref_label);
        }

        let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        info_box.set_valign(gtk4::Align::Center);
        info_box.set_hexpand(true);

        let filename = std::path::Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        let name_label = gtk4::Label::new(Some(filename));
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        name_label.set_max_width_chars(40);
        name_label.set_halign(gtk4::Align::Start);
        if *is_ref {
            name_label.set_css_classes(&["ref-path"]);
        }

        let path_label = gtk4::Label::new(Some(path));
        path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        path_label.set_max_width_chars(40);
        path_label.set_halign(gtk4::Align::Start);
        path_label.set_css_classes(&["dim-label"]);

        let res_str = image::image_dimensions(std::path::Path::new(path))
            .ok().map(|(w, h)| format!("{}x{}", w, h)).unwrap_or_default();
        let size_val = std::fs::metadata(path).ok().map(|m| m.len()).unwrap_or(0);
        let size_str = preview::format_size(size_val);
        let date_str = read_exif_date(path);

        let meta_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
        meta_box.set_halign(gtk4::Align::Start);
        if let Some(ref d) = date_str {
            let date_label = gtk4::Label::new(Some(d));
            date_label.set_css_classes(&["dim-label"]);
            meta_box.append(&date_label);
            let dot = gtk4::Label::new(Some("\u{00b7}"));
            dot.set_css_classes(&["dim-label"]);
            meta_box.append(&dot);
        }
        if !res_str.is_empty() {
            let res_label = gtk4::Label::new(Some(&res_str));
            res_label.set_css_classes(&["dim-label"]);
            meta_box.append(&res_label);
            let dot = gtk4::Label::new(Some("\u{00b7}"));
            dot.set_css_classes(&["dim-label"]);
            meta_box.append(&dot);
        }
        let size_label = gtk4::Label::new(Some(&size_str));
        size_label.set_css_classes(&["dim-label"]);
        meta_box.append(&size_label);

        if *is_ref {
            let dot = gtk4::Label::new(Some("\u{00b7}"));
            dot.set_css_classes(&["dim-label"]);
            meta_box.append(&dot);
            let ref_badge = gtk4::Label::new(Some("REF"));
            ref_badge.set_css_classes(&["status-pill"]);
            meta_box.append(&ref_badge);
        }

        info_box.append(&name_label);
        info_box.append(&path_label);
        info_box.append(&meta_box);

        let actions_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        actions_box.set_valign(gtk4::Align::Center);

        let move_btn = btn_icon_text("Move", "go-jump-symbolic");
        move_btn.set_css_classes(&["small"]);
        let path_for_move = path.clone();
        let sl = status_label.clone();
        let pw = window.clone();
        let move_btn_ref = move_btn.clone();
        let mw_move = main_widgets.clone();
        let picture_move = picture.clone();
        let overlay_move = status_overlay_box.clone();
        let icon_move = status_icon.clone();
        let overlay_lbl_move = status_overlay_label.clone();
        let moved_to_lbl_move = moved_to_label.clone();
        let path_lbl_move = path_label.clone();
        move_btn.connect_clicked(move |_| {
            let mw = mw_move.clone();
            let p = path_for_move.clone();
            let p2 = p.clone();
            let pic = picture_move.clone();
            let ov = overlay_move.clone();
            let ic = icon_move.clone();
            let ol = overlay_lbl_move.clone();
            let mtl = moved_to_lbl_move.clone();
            let pl = path_lbl_move.clone();
            move_single_file_with_callback(&p, &sl, &pw, &move_btn_ref, move |dest_path| {
                // Switch thumbnail to moved icon + new path
                pic.set_visible(false);
                ic.set_icon_name(Some("go-jump-symbolic"));
                ol.set_text("Moved to:");
                mtl.set_text(&dest_path);
                mtl.set_visible(true);
                ov.set_visible(true);
                pl.set_text(&dest_path);
                // Update main window
                if let Some(w) = mw.get(&p2) {
                    let msg = format!("→ Moved to {}", dest_path);
                    w.moved_label.set_text(&msg);
                    w.moved_label.set_visible(true);
                    w.move_btn.set_visible(false);
                }
            });
        });

        let trash_btn = btn_icon_text("Trash", "user-trash-symbolic");
        trash_btn.set_css_classes(&["small", "destructive-action"]);
        let path_for_trash = path.clone();
        let sl2 = status_label.clone();
        let trash_btn_ref = trash_btn.clone();
        let mw_trash = main_widgets.clone();
        let picture_trash = picture.clone();
        let overlay_trash = status_overlay_box.clone();
        let icon_trash = status_icon.clone();
        let overlay_lbl_trash = status_overlay_label.clone();
        trash_btn.connect_clicked(move |_| {
            if trash::delete(&path_for_trash).is_ok() {
                sl2.set_text(&format!("🗑 Trashed: {}", path_for_trash));
                // Switch thumbnail to trash icon
                picture_trash.set_visible(false);
                icon_trash.set_icon_name(Some("user-trash-symbolic"));
                overlay_lbl_trash.set_text("Moved to trash");
                overlay_trash.set_visible(true);
                trash_btn_ref.set_visible(false);
                // Update main window
                if let Some(w) = mw_trash.get(&path_for_trash) {
                    w.deleted_label.set_visible(true);
                    w.trash_btn.set_visible(false);
                    w.restore_btn.set_visible(true);
                    if w.reference {
                        w.row.set_css_classes(&["deleted", "ref-row"]);
                    } else {
                        w.row.set_css_classes(&["deleted"]);
                    }
                }
            } else {
                sl2.set_text(&format!("Failed to trash: {}", path_for_trash));
            }
        });

        actions_box.append(&move_btn);
        actions_box.append(&trash_btn);

        row.append(&image_box);
        row.append(&info_box);
        row.append(&actions_box);
        main_box.append(&row);
    }

    main_box.append(&status_label);
    preview_viewport.set_child(Some(&main_box));
    scrolled.set_child(Some(&preview_viewport));
    window.set_child(Some(&scrolled));
    window.present();
}

fn show_about_window(parent: &gtk4::Window) {
    let about = libadwaita::AboutDialog::builder()
        .application_name("ImpHash")
        .application_icon("imphash")
        .version(APP_VERSION)
        .comments("Duplicate Image Finder - finds near-duplicate images using perceptual hashing")
        .developer_name("antoxa78")
        .license_type(gtk4::License::MitX11)
        .website("https://github.com/antoxa78/ImpHash")
        .issue_url("https://github.com/antoxa78/ImpHash/issues")
        .copyright("\u{00a9} 2026 antoxa78")
        .build();
    about.present(Some(parent));
}
