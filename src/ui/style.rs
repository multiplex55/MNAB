//! Theme-derived semantic colors and density-derived geometry for the UI.
//!
//! Widgets should consume these roles rather than choosing colors or maintaining
//! compact/comfortable layout branches of their own.

use crate::app::settings::DisplayDensity;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemanticTokens {
    pub background: egui::Color32,
    pub panel: egui::Color32,
    pub sidebar: egui::Color32,
    pub header: egui::Color32,
    pub border: egui::Color32,
    pub hover: egui::Color32,
    pub selection: egui::Color32,
    pub muted_text: egui::Color32,
    pub positive_money: egui::Color32,
    pub negative_money: egui::Color32,
    pub warning: egui::Color32,
    pub error: egui::Color32,
    pub ready_to_assign: egui::Color32,
    pub funded: egui::Color32,
    pub underfunded: egui::Color32,
    pub overspent: egui::Color32,
}

impl SemanticTokens {
    /// Derives the complete hierarchy from the active egui theme. The accents are
    /// deliberately generic status hues, not a copy of another product's palette.
    #[must_use]
    pub fn from_visuals(visuals: &egui::Visuals) -> Self {
        let dark = visuals.dark_mode;
        let (background, panel, sidebar, header, border, muted_text) = if dark {
            (
                egui::Color32::from_rgb(20, 23, 28),
                egui::Color32::from_rgb(29, 33, 40),
                egui::Color32::from_rgb(24, 28, 35),
                egui::Color32::from_rgb(34, 39, 47),
                egui::Color32::from_rgb(61, 68, 79),
                egui::Color32::from_rgb(163, 171, 184),
            )
        } else {
            (
                egui::Color32::from_rgb(246, 247, 249),
                egui::Color32::WHITE,
                egui::Color32::from_rgb(238, 241, 245),
                egui::Color32::from_rgb(250, 251, 252),
                egui::Color32::from_rgb(207, 212, 220),
                egui::Color32::from_rgb(94, 103, 117),
            )
        };
        Self {
            background,
            panel,
            sidebar,
            header,
            border,
            hover: if dark {
                egui::Color32::from_rgb(48, 55, 67)
            } else {
                egui::Color32::from_rgb(226, 231, 238)
            },
            selection: if dark {
                egui::Color32::from_rgb(43, 76, 105)
            } else {
                egui::Color32::from_rgb(205, 228, 246)
            },
            muted_text,
            positive_money: if dark {
                egui::Color32::from_rgb(103, 202, 146)
            } else {
                egui::Color32::from_rgb(25, 126, 76)
            },
            negative_money: if dark {
                egui::Color32::from_rgb(245, 126, 128)
            } else {
                egui::Color32::from_rgb(188, 49, 55)
            },
            warning: if dark {
                egui::Color32::from_rgb(241, 190, 83)
            } else {
                egui::Color32::from_rgb(155, 100, 8)
            },
            error: if dark {
                egui::Color32::from_rgb(255, 112, 116)
            } else {
                egui::Color32::from_rgb(181, 35, 42)
            },
            ready_to_assign: if dark {
                egui::Color32::from_rgb(99, 181, 225)
            } else {
                egui::Color32::from_rgb(25, 112, 164)
            },
            funded: if dark {
                egui::Color32::from_rgb(90, 190, 132)
            } else {
                egui::Color32::from_rgb(26, 126, 76)
            },
            underfunded: if dark {
                egui::Color32::from_rgb(231, 181, 78)
            } else {
                egui::Color32::from_rgb(151, 96, 5)
            },
            overspent: if dark {
                egui::Color32::from_rgb(244, 107, 112)
            } else {
                egui::Color32::from_rgb(184, 39, 47)
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub register_row_height: f32,
    pub category_row_height: f32,
    pub toolbar_height: f32,
    pub panel_padding: f32,
    pub section_spacing: f32,
    pub corner_radius: f32,
}

impl Metrics {
    #[must_use]
    pub const fn for_density(density: DisplayDensity) -> Self {
        match density {
            DisplayDensity::Compact => Self {
                register_row_height: 26.0,
                category_row_height: 26.0,
                toolbar_height: 38.0,
                panel_padding: 8.0,
                section_spacing: 10.0,
                corner_radius: 4.0,
            },
            DisplayDensity::Normal => Self {
                register_row_height: 32.0,
                category_row_height: 32.0,
                toolbar_height: 44.0,
                panel_padding: 12.0,
                section_spacing: 16.0,
                corner_radius: 6.0,
            },
            DisplayDensity::Comfortable => Self {
                register_row_height: 40.0,
                category_row_height: 40.0,
                toolbar_height: 52.0,
                panel_padding: 16.0,
                section_spacing: 22.0,
                corner_radius: 8.0,
            },
        }
    }
}

pub fn apply(ctx: &egui::Context, density: DisplayDensity) {
    let tokens = SemanticTokens::from_visuals(&ctx.style().visuals);
    let metrics = Metrics::for_density(density);
    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing =
        egui::vec2(metrics.panel_padding / 1.5, metrics.panel_padding / 2.0);
    style.spacing.interact_size.y = metrics.register_row_height;
    style.visuals.panel_fill = tokens.background;
    style.visuals.window_fill = tokens.panel;
    style.visuals.widgets.hovered.weak_bg_fill = tokens.hover;
    style.visuals.selection.bg_fill = tokens.selection;
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_is_one_monotonic_metric_mapping() {
        let compact = Metrics::for_density(DisplayDensity::Compact);
        let normal = Metrics::for_density(DisplayDensity::Normal);
        let comfortable = Metrics::for_density(DisplayDensity::Comfortable);
        assert!(
            compact.register_row_height < normal.register_row_height
                && normal.register_row_height < comfortable.register_row_height
        );
        assert!(
            compact.category_row_height < normal.category_row_height
                && normal.category_row_height < comfortable.category_row_height
        );
        assert!(
            compact.toolbar_height < normal.toolbar_height
                && normal.toolbar_height < comfortable.toolbar_height
        );
        assert!(
            compact.panel_padding < normal.panel_padding
                && normal.panel_padding < comfortable.panel_padding
        );
        assert!(
            compact.section_spacing < normal.section_spacing
                && normal.section_spacing < comfortable.section_spacing
        );
        assert!(
            compact.corner_radius < normal.corner_radius
                && normal.corner_radius < comfortable.corner_radius
        );
    }

    #[test]
    fn light_and_dark_have_complete_distinct_semantic_roles() {
        let light = SemanticTokens::from_visuals(&egui::Visuals::light());
        let dark = SemanticTokens::from_visuals(&egui::Visuals::dark());
        assert_ne!(light, dark);
        for tokens in [light, dark] {
            let roles = [
                tokens.background,
                tokens.panel,
                tokens.sidebar,
                tokens.header,
                tokens.border,
                tokens.hover,
                tokens.selection,
                tokens.muted_text,
                tokens.positive_money,
                tokens.negative_money,
                tokens.warning,
                tokens.error,
                tokens.ready_to_assign,
                tokens.funded,
                tokens.underfunded,
                tokens.overspent,
            ];
            assert!(
                roles
                    .into_iter()
                    .all(|color| color != egui::Color32::TRANSPARENT)
            );
        }
    }
}
