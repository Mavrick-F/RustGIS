//! Compact attribute-table geometry calculator, bound to the table's layer.
use crate::{RustGisApp, theme::Tokens};
use rustgis_engine::{
    Session,
    data::FieldType,
    geometry_calc::{properties, units},
};
use serde_json::json;

/// The state of the Calculate Geometry dialog.
pub struct Form {
    pub layer: u32,
    property: String,
    unit: String,
    field: String,
    field_edited: bool,
    create: bool,
    selected: bool,
    overwrite: bool,
    epsg: String,
    error: Option<String>,
}

impl Form {
    /// A form for `layer`, writing its first property to a new, uniquely named field.
    pub fn new(s: &Session, layer: u32) -> Result<Self, String> {
        let l = s.map().layer(layer).ok_or_else(|| tl!("Layer not found").to_string())?;
        let property = properties(l.geometry_type()).first().map(|v| v.0).ok_or_else(|| tl!("No geometry to calculate").to_string())?;
        let unit = units(property).first().map(|v| v.0).unwrap_or("m");
        let base = format!("{property}_{unit}");
        let mut field = base.clone();
        let mut n = 2;
        while l.dataset.field_index(&field).is_some() {
            field = format!("{base}_{n}");
            n += 1;
        }
        Ok(Self {
            layer,
            property: property.into(),
            unit: unit.into(),
            field,
            field_edited: false,
            create: true,
            selected: !l.selection.is_empty(),
            overwrite: false,
            epsg: "32650".into(),
            error: None,
        })
    }
}

/// A combo box over `choices` (`(key, label)` pairs) editing the key `value`; true when changed.
fn choice(ui: &mut egui::Ui, id: &str, value: &mut String, choices: &[(&str, &str)], width: f32, label_id: egui::Id) -> bool {
    let mut changed = false;
    let label = choices.iter().find(|(k, _)| *k == value).map(|v| v.1).unwrap_or("");
    egui::ComboBox::from_id_salt(id)
        .selected_text(tl!(label))
        .width(width)
        .show_ui(ui, |ui| {
            for (key, label) in choices {
                changed |= ui.selectable_value(value, (*key).into(), tl!(*label)).changed();
            }
        })
        .response
        .labelled_by(label_id);
    changed
}

/// Shows the dialog while it is open and runs `edit.calculateGeometry` on Calculate.
pub fn show(app: &mut RustGisApp, ctx: &egui::Context) {
    let Some(mut f) = app.dialogs.geometry.take() else { return };
    let Some(l) = app.session.map().layer(f.layer) else {
        app.dialogs.open = None;
        return;
    };
    let name = l.name.clone();
    let props = properties(l.geometry_type());
    let fields: Vec<_> = l.dataset.fields.iter().filter(|f| f.ty.is_numeric()).map(|f| (f.name.clone(), f.ty)).collect();
    let selected_count = l.selection.len();
    let total = l.dataset.features.len();
    let mut open = true;
    let mut close = false;
    let mut calculate = false;
    let t = Tokens::get(ctx);
    egui::Window::new(tl!("Calculate Geometry"))
        .id(egui::Id::unique("geometry_calculator_compact"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .auto_sized()
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center() + egui::vec2(0.0, -40.0))
        .show(ctx, |ui| {
            let width = (ctx.content_rect().width() - 48.0).clamp(240.0, 320.0);
            let control_width = width - 80.0;
            ui.set_width(width);
            ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
            {
                egui::Grid::new("geometry_form").num_columns(2).min_col_width(72.0).spacing(egui::vec2(8.0, 8.0)).show(ui, |ui| {
                    ui.label(tl!("Layer"));
                    ui.label(&name);
                    ui.end_row();
                    let previous = (f.property.clone(), f.unit.clone());
                    let property_label = ui.label(tl!("Property"));
                    if choice(ui, "geom_property", &mut f.property, props, control_width, property_label.id) {
                        f.unit = units(&f.property).first().map(|v| v.0).unwrap_or("m").into();
                        f.overwrite = false;
                    }
                    ui.end_row();
                    let unit_label = ui.label(tl!("Unit"));
                    if choice(ui, "geom_unit", &mut f.unit, units(&f.property), control_width, unit_label.id) {
                        f.overwrite = false;
                    }
                    ui.end_row();
                    if f.create && !f.field_edited && previous != (f.property.clone(), f.unit.clone()) {
                        let base = format!("{}_{}", f.property, f.unit);
                        f.field = base.clone();
                        let mut n = 2;
                        while l.dataset.field_index(&f.field).is_some() {
                            f.field = format!("{base}_{n}");
                            n += 1;
                        }
                    }
                    if f.unit == "projected" {
                        let input_label = ui.label(tl!("EPSG code"));
                        if ui
                            .add(egui::TextEdit::singleline(&mut f.epsg).desired_width(control_width))
                            .labelled_by(input_label.id)
                            .changed()
                        {
                            f.overwrite = false;
                        }
                        ui.end_row();
                    }
                    ui.label(tl!("Output"));
                    ui.horizontal(|ui| {
                        if ui.radio_value(&mut f.create, true, tl!("New field")).changed() {
                            f.overwrite = false;
                            f.field_edited = false;
                            let base = format!("{}_{}", f.property, f.unit);
                            f.field = base.clone();
                            let mut n = 2;
                            while l.dataset.field_index(&f.field).is_some() {
                                f.field = format!("{base}_{n}");
                                n += 1;
                            }
                            f.error = None;
                        }
                        if ui.add_enabled(!fields.is_empty(), egui::RadioButton::new(!f.create, tl!("Existing field"))).clicked() {
                            f.create = false;
                            f.overwrite = false;
                            if !fields.iter().any(|(n, _)| *n == f.field) {
                                f.field = fields.first().map(|v| v.0.clone()).unwrap_or_default();
                            }
                        }
                    });
                    ui.end_row();
                    let input_label = ui.label(tl!("Field"));
                    if f.create {
                        f.field_edited |= ui
                            .add(egui::TextEdit::singleline(&mut f.field).hint_text(tl!("Field name")).desired_width(control_width))
                            .changed();
                    } else {
                        egui::ComboBox::from_id_salt("geom_field")
                            .selected_text(&f.field)
                            .width(control_width)
                            .show_ui(ui, |ui| {
                                for (name, _) in &fields {
                                    if ui.selectable_value(&mut f.field, name.clone(), name).changed() {
                                        f.overwrite = false;
                                    }
                                }
                            })
                            .response
                            .labelled_by(input_label.id);
                    }
                    ui.end_row();
                    ui.label(tl!("Features"));
                    ui.horizontal(|ui| {
                        if ui.radio_value(&mut f.selected, false, format!("{} ({total})", tl!("All"))).changed() {
                            f.overwrite = false;
                        }
                        let label = format!("{} ({selected_count})", tl!("Selected"));
                        if ui.add_enabled(selected_count > 0, egui::RadioButton::new(f.selected, label)).clicked() {
                            f.selected = true;
                            f.overwrite = false;
                        }
                    });
                    ui.end_row();
                });
                if !f.create && fields.iter().any(|(name, ty)| *name == f.field && *ty == FieldType::Integer) {
                    ui.label(egui::RichText::new(tl!("Integer field: values are rounded.")).small().weak());
                }
                if !f.create {
                    ui.checkbox(&mut f.overwrite, tl!("Overwrite existing values"));
                }
                if let Some(error) = &f.error {
                    ui.colored_label(t.red, error);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        calculate = ui
                            .add_enabled_ui(
                                !f.field.trim().is_empty() && (f.create || f.overwrite) && (!f.selected || selected_count > 0),
                                |ui| crate::widgets::dialog_button(ui, tl!("Calculate"), Some(t.accent), egui::vec2(84.0, 28.0)).clicked(),
                            )
                            .inner;
                        close = crate::widgets::dialog_button(ui, tl!("Cancel"), None, egui::vec2(84.0, 28.0)).clicked();
                    });
                });
            }
        });
    let epsg = f.epsg.trim().parse::<u32>().ok();
    if calculate && f.unit == "projected" && epsg.is_none() {
        f.error = Some(tl!("Enter an EPSG code as a number, such as 32650").to_string());
    } else if calculate {
        let p = json!({"layer": f.layer, "field": f.field.trim(), "create": f.create, "overwrite": f.overwrite,
            "property": f.property, "unit": f.unit, "epsg": epsg, "scope": if f.selected { "selected" } else { "all" }});
        match app.run("edit.calculateGeometry", p) {
            Ok(result) => {
                app.session.notify(crate::i18n::fmt("Calculated {rows} features", &[("rows", &result["rows"].to_string())]), false);
                close = true;
            }
            Err(error) => f.error = Some(error),
        }
    }
    if !open || close {
        app.dialogs.open = None;
    } else {
        app.dialogs.geometry = Some(f);
    }
}
