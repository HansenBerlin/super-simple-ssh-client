use ratatui::layout::Constraint;

pub(crate) const HELP_TEXT: &str = "(t)erminal | (u)pload | (d)ownload | (o)ptions | (v)iew | (q)uit";

pub(crate) const LABEL_WIDTH: usize = 9;
pub(crate) const TRANSFER_PICKER_WIDTH: u16 = 60;
pub(crate) const TRANSFER_PICKER_HEIGHT: u16 = 90;

pub(crate) const HEADER_HEIGHT: u16 = 3;
pub(crate) const TERMINAL_FOOTER_HEIGHT: u16 = 2;

pub(crate) const HELP_COLUMN_PERCENTAGES: [u16; 3] = [33, 33, 34];
pub(crate) const COMPACT_COLUMN_PERCENTAGES: [u16; 2] = [50, 50];

pub(crate) const MODAL_WIDTH_PERCENT: u16 = 70;
pub(crate) const MODAL_MAX_HEIGHT_PERCENT: u16 = 70;
pub(crate) const MODAL_MIN_WIDTH: u16 = 30;

pub(crate) const TRANSFER_CONFIRM_WIDTH_PERCENT: u16 = 70;

pub(crate) const PICKER_FOOTER_HEIGHT: u16 = 2;

pub(crate) const KEY_PICKER_WIDTH: u16 = 70;
pub(crate) const KEY_PICKER_HEIGHT: u16 = 60;

pub(crate) const POPUP_MIN_WIDTH: u16 = 10;
pub(crate) const POPUP_MIN_HEIGHT: u16 = 5;

pub(crate) fn help_columns() -> [Constraint; 3] {
    HELP_COLUMN_PERCENTAGES.map(Constraint::Percentage)
}

pub(crate) fn compact_columns() -> [Constraint; 2] {
    COMPACT_COLUMN_PERCENTAGES.map(Constraint::Percentage)
}
