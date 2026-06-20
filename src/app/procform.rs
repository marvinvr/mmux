//! State for the "+ New Process" guided form — the modal wizard raised from the
//! PROCESSES section. It walks one field per screen (name → command → working dir
//! → review), then the collected values are written to the project's `mmux.yaml`
//! by [`App::finish_new_process`](super::App). Keys are driven in
//! [`App::procform_key`](super::input) and it's drawn in
//! [`view::git::render_procform`](super::view).

/// Which field the form is editing. `Review` is the final screen: it shows the
/// gathered values and toggles autostart before writing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Name,
    Command,
    Cwd,
    Review,
}

/// Total steps, for the "Step N of 4" header.
pub(crate) const STEPS: usize = 4;

pub(crate) struct ProcForm {
    /// The project whose config the process is written to.
    pub project: usize,
    pub step: Step,
    pub name: String,
    pub command: String,
    pub cwd: String,
    pub autostart: bool,
    /// The edit buffer for the active text step; committed into the matching field
    /// when advancing (see [`super::input`]).
    pub buf: String,
    /// A validation message shown under the input (empty/duplicate name, …).
    pub error: Option<String>,
}

impl ProcForm {
    pub(crate) fn new(project: usize) -> ProcForm {
        ProcForm {
            project,
            step: Step::Name,
            name: String::new(),
            command: String::new(),
            cwd: String::new(),
            autostart: false,
            buf: String::new(),
            error: None,
        }
    }

    /// The current step's 1-based index, for the header.
    pub(crate) fn step_index(&self) -> usize {
        match self.step {
            Step::Name => 1,
            Step::Command => 2,
            Step::Cwd => 3,
            Step::Review => 4,
        }
    }
}
