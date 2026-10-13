//! Editing a field in the config pane: filing an edit, undoing it, and the
//! batch that goes out on close.

#[cfg(test)]
mod choice_list;
mod close_reread;
mod confirm_field;
mod cycle_field;
mod dispatch_writes;
mod file_edit;
#[cfg(test)]
mod paste;
mod restore_default;
#[cfg(test)]
mod union_edit;
