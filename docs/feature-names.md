# Feature naming

FigCraft describes itself as an **interface design tool**, without the comparative Figma-class tag. Interface-size command help describes the size steps directly. Command IDs, file-format interoperability references, behavioral documentation and attribution remain unchanged.

Regression tests prevent the retired comparative display phrases from returning to MCP instructions or command help. This copy change makes no legal claim about the comparative terms and does not change behavior.

## Developer Mode

**Old label:** "Dev Mode". **New label:** "Developer Mode", since #2135.

Figma holds a US registration for DEV MODE ([US 7574348](https://tsdr.uspto.gov/statusview/sn98045640), registered 2024-11-19, owner Figma, Inc.), so neither a label nor a translation calls the mode by the old name. The identifiers keep their names (`view.devMode`, `devMode.*`), and so do the preference keys and saved document fields; the Actions palette still finds the command by the old name. Some translations had a generic "dev mode" where the proper name belongs (French "Mode Dev", Portuguese "Modo Dev", Dutch "Dev Mode"); they now read "Mode développeur", "Modo do desenvolvedor" and "Ontwikkelaarsmodus", so every language names the mode the same way. `crates/ui-egui/src/naming_tests.rs` keeps the retired name out of the labels and the catalogs.
