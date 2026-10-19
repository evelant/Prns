/// The qualified Xtensa toolchain emits unaligned outgoing argument frames when
/// frame pointers are forced. Check Cargo's effective tokenized flags, including
/// their ordering, instead of trusting a higher-level RUSTFLAGS environment value.
pub fn permits_qualification(enabled: bool, target_arch: &str, encoded_flags: &str) -> bool {
    if !enabled || target_arch != "xtensa" {
        return true;
    }

    let mut disabled = false;
    let mut arguments = encoded_flags.split('\u{1f}');
    while let Some(argument) = arguments.next() {
        let option = if argument == "-C" {
            arguments.next()
        } else {
            argument.strip_prefix("-C")
        };
        if let Some(option) = option {
            if option == "force-frame-pointers" || option.starts_with("force-frame-pointers=") {
                disabled = option == "force-frame-pointers=no";
            }
        }
    }
    disabled
}

#[cfg(test)]
mod tests {
    use super::permits_qualification;

    #[test]
    fn normal_builds_are_unaffected() {
        assert!(permits_qualification(false, "xtensa", ""));
        assert!(permits_qualification(
            false,
            "xtensa",
            "-Cforce-frame-pointers=yes"
        ));
        assert!(permits_qualification(true, "riscv32", ""));
    }

    #[test]
    fn qualification_requires_explicit_no() {
        for flags in [
            "",
            "-Cforce-frame-pointers",
            "-Cforce-frame-pointers=yes",
            "-Cforce-frame-pointers=invalid",
        ] {
            assert!(!permits_qualification(true, "xtensa", flags), "{flags}");
        }
        assert!(permits_qualification(
            true,
            "xtensa",
            "-Cforce-frame-pointers=no"
        ));
        assert!(permits_qualification(
            true,
            "xtensa",
            "-C\u{1f}force-frame-pointers=no"
        ));
    }

    #[test]
    fn last_relevant_option_wins_across_both_spellings() {
        assert!(permits_qualification(true, "xtensa", "-C\u{1f}force-frame-pointers\u{1f}-Clink-arg=-Tlinkall.x\u{1f}-Cforce-frame-pointers=no"));
        assert!(!permits_qualification(
            true,
            "xtensa",
            "-Cforce-frame-pointers=no\u{1f}-C\u{1f}force-frame-pointers"
        ));
        assert!(!permits_qualification(
            true,
            "xtensa",
            "-Cforce-frame-pointers=no\u{1f}-Cforce-frame-pointers=yes"
        ));
        assert!(!permits_qualification(
            true,
            "xtensa",
            "-Clink-arg=force-frame-pointers=no"
        ));
    }
}
