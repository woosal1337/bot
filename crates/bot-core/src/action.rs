#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Action {
    OpenCommandPalette,
    OpenProviderPicker,
    OpenAccountPicker,
    OpenModelPicker,
    OpenEffortPicker,
    FocusComposer,
    SubmitComposer,
    CancelTurn,
    ToggleHelp,
    Quit,
}
