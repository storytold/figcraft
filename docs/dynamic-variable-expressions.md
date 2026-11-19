# Dynamic variable expression syntax

Enter an expression in a variable's **Edit variable** window. Enter only the expression, not an
assignment such as `StrokeThickness = ...`.

```text
{CornerRadius} * 2
if {CornerRadius} == 1 then 20 else 10
not {MenuOpen}
lerp(hsl(1, 100, 100), hex(#000000), 0.15)
{isElegableUser} and "Success" or "Fail"
```

## Values and references

Expressions can use:

| Kind | Syntax | Example |
|---|---|---|
| Number | Integers, decimals, and exponents | `12`, `0.5`, `2.5e1` |
| Boolean | `true`, `false` | `not {MenuOpen}` |
| Text | Single- or double-quoted strings | `"Hello " + {FirstName}` |
| Color | A color variable, `hsl(h, s, l)`, or `hex(#RRGGBB)` | `{BrandColor}`, `hsl(1, 100, 100)`, `hex(#111111)` |
| Variable | `{name}` or `{id}` | `{CornerRadius}`, `{Brand color}` |

Every variable reference must be enclosed in braces, including ordinary names, names that contain
spaces or symbols, and names that collide with literals, operators, or functions such as `and`,
`hex`, and `lerp`. The Variables editor offers matching variables while typing inside `{…}`; use
Up/Down, Enter or Tab, or click a suggestion. Outside braces, typing a syntax prefix offers
operators (`and`, `or`, `not`), keywords (`if`, `then`, `else`), and color functions (`hsl`, `hex`,
`lerp`) with their argument hints. Accepting a variable inserts the closing brace and a trailing
space; accepting an operator also leaves the caret after its trailing space. Choosing a function
inserts its parentheses and leaves the caret inside. The popup sits beside the caret. Typing `(` or
`{`, `[`, a quote, or a backtick inserts its matching closer and keeps the caret between the pair.
When text is selected, typing one of these openers wraps the selection instead of replacing it.
References are saved by variable ID, so renaming a referenced variable does not change which
variable the formula reads.

The editor syntax-highlights braced variable references, boolean operators, and color functions
using FigCraft's native theme colors.
Whitespace is allowed around function names, parentheses, commas, and arguments: for example,
`hsl ( 1 , 100 , 100 )` and `hex ( #111111 )`. Dynamic expressions require the explicit `hex(...)`
form; they do not accept bare `#RRGGBB` color literals.

In `hsl(h, s, l)`, hue is in degrees and wraps by 360 degrees (so `hsl(360, 100, 50)` is the same
color as `hsl(0, 100, 50)`). Saturation and lightness are percentages clamped to 0..100. The result
is opaque.

## Operators

| Operation | Syntax | Rules |
|---|---|---|
| Conditional | `if condition then value else value` | Condition must be boolean; both branches must have the same type. |
| Boolean OR | `or`, `\|\|` | Boolean operands. |
| Boolean AND | `and`, `&&` | Boolean operands. |
| Boolean NOT | `not`, `!` | Boolean operand. |
| Equality | `==`, `=`, `!=` | Compare values of the same type, including colors. |
| Ordering | `<`, `<=`, `>`, `>=` | Compare two numbers or two text values. |
| Addition | `+` | Adds numbers or joins text. |
| Subtraction | `-` | Numbers only. |
| Multiplication | `*` | Numbers only. |
| Division | `/` | Numbers only; division by zero produces no result, leaving the current variable value unchanged. |
| Unary negation | `-value` | Negates a number; on a boolean it inverts the value. Prefer `not` for boolean clarity. |
| Color interpolation | `lerp(color1, color2, t)` | Interpolates in sRGB; `t` is clamped to 0..1. |

Parentheses group expressions. From loosest to tightest, operators are conditional, `or`, `and`,
`not`, comparisons, `+`/`-`, `*`/`/`, and unary `-`.
The editor displays expressions with only the parentheses needed to preserve operator precedence;
it does not add redundant parentheses around comparisons or boolean groups.
For conditional selection, `condition and value or fallback` is also supported when the condition
is boolean and both non-boolean values have the same type; it selects `value` when the condition is
true and `fallback` otherwise. There is no Lua-style truthiness fallback: an empty string or `0`
is still selected when it is the matching `value`. Use `if condition then value else fallback`
when a branch may be empty or zero, since it makes the two outcomes explicit. Boolean `and`/`or`
expressions retain their logical meaning. For example, group a mixed condition with parentheses:

```text
if ({var1} or {var2}) and {var 3} then "Success" else "Fail"
```

## Type and color rules

The expression's result must match the variable's type. A string variable can also use a number or
boolean result, which is converted to text. Conditions must be boolean, and conditional branches
must agree in type. Invalid types and dependency cycles are rejected.

Color variables can be referenced directly, selected by a conditional, and compared for equality.
Dynamic expressions ignore color alpha: references and hex literals are treated as fully opaque,
color equality compares the visible RGB values, and `lerp` interpolates RGB with an opaque result.
It accepts a color variable, an HSL color, or a hex color for each endpoint:

```text
if {UseDarkTheme} then {DarkSurface} else {LightSurface}
lerp(hsl(1, 100, 100), {BrandColor}, 0.15)
lerp(hex(#111111), hex(#0000FF), 0.5)
```

For a color variable with an expression, edit the percentage at the right side of its value cell to
set the output opacity for that mode. It is separate from the formula, remains applied when the
expression recomputes, and defaults to 100%. Removing the override restores 100% opacity. FigCraft's
separate color calculations also support alpha replacement and color mixing.

Clearing an expression leaves its most recently computed value as a regular static value. If the
variable previously linked to another variable, clearing the expression does not restore that link.
Deleting a variable referenced by a formula clears that formula and keeps its last computed value
as a static value, so the document does not retain a dangling reference.

## Limitations

Expressions do not support comments, user-defined functions, or assignment syntax. Although the
shared expression parser has mode-specific forms such as `Accent[Dark]` and `mode(Theme)`, those
forms are not enabled for dynamic variable formulas. Each formula is evaluated for the output
variable's modes, and updates synchronously when variables change, including in prototype playback.
