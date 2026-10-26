# Taiwan Traditional Chinese draft

Adds a baked-in Taiwan catalog for menus, command labels, settings, tooltips and interface text.
Traditional locale tags resolve to it; explicit scripts override regions. Desktop fallback puts
Taiwan glyphs ahead of Simplified and Japanese faces. No new fonts are shipped.

The machine-assisted draft uses OpenCC s2twp on the project's Simplified catalog, with Taiwan
terminology corrections and translations of missing English UI entries. Native review is pending.
There is no conversion dependency or mandatory cross-language parity check in the project.

Tests cover locale selection, Taiwan terms, substitutions, Chinese plurals and font preference;
existing catalog tests validate keys and placeholders. Free-form engine errors retain fallback.
