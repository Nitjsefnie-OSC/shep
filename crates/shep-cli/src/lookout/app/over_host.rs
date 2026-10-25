//! What a lookout over `--host` will not do: the screens that read or write
//! this machine's files.

use super::*;

/// The one sentence for a key, a pane or a feed that needs this machine.
///
/// Plain words and no key names: the bleats feed draws it as its only line
/// and the status bar raises it as a notice, and both render on one screen.
pub(crate) const NOT_OVER_HOST: &str =
    "not available over --host: it reads this machine's files, not the host's";

impl App {
    /// Whether this lookout watches a host. When it does, raises
    /// [`NOT_OVER_HOST`] as the notice, and the caller returns
    /// [`Effect::None`].
    ///
    /// The settings screen, the secrets pane and the dog panes read
    /// `shep.toml`, `dogs.toml`, the secret store and dog binaries on this
    /// machine, which would be shown under the host's name.
    pub(super) fn refuse_over_host(&mut self) -> bool {
        if self.remote_host.is_none() {
            return false;
        }
        self.notice = Some(Notice::grave(NOT_OVER_HOST));
        true
    }
}
