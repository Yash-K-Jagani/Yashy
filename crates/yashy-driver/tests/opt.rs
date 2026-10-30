use yashy_driver::{resolve_opt, CommandKind, OptLevel};

#[test]
fn debug_build_keeps_the_o2_default() {
    assert_eq!(resolve_opt(CommandKind::Build, false, None), OptLevel::Full);
}

#[test]
fn run_defaults_to_o0() {
    assert_eq!(resolve_opt(CommandKind::Run, false, None), OptLevel::None);
}

#[test]
fn test_defaults_to_o0() {
    assert_eq!(resolve_opt(CommandKind::Test, false, None), OptLevel::None);
}

#[test]
fn release_build_defaults_to_o3() {
    assert_eq!(resolve_opt(CommandKind::Build, true, None), OptLevel::Aggressive);
}

#[test]
fn an_explicit_level_wins_over_release() {
    assert_eq!(resolve_opt(CommandKind::Build, true, Some(OptLevel::None)), OptLevel::None);
    assert_eq!(resolve_opt(CommandKind::Build, true, Some(OptLevel::Less)), OptLevel::Less);
    assert_eq!(resolve_opt(CommandKind::Build, true, Some(OptLevel::Full)), OptLevel::Full);
    assert_eq!(
        resolve_opt(CommandKind::Build, true, Some(OptLevel::Aggressive)),
        OptLevel::Aggressive
    );
}
