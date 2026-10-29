//! MCP prompts: ready-made design workflows that tell an agent which tools and commands to use.

use serde_json::{Value, json};

pub struct PromptArg {
    pub name: &'static str,
    pub description: &'static str,
    pub required: bool,
}

pub struct PromptDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub args: &'static [PromptArg],
    /// Template; `{arg}` is replaced by the argument (or its default).
    pub template: &'static str,
    pub defaults: &'static [(&'static str, &'static str)],
}

const VERIFY: &str = "\n\nVerify as you go: after each major step call inspect_document (or get_node on the frame) and check sizes, fills, layout and text; call screenshot at the end and look at it. Fix anything that is off before finishing, then report the ids of the frames you made.";

pub const PROMPTS: &[PromptDef] = &[
    PromptDef {
        name: "mobile_screen",
        title: "Design a mobile screen",
        description: "Build one mobile app screen in a phone frame with auto layout, real text and a clear hierarchy.",
        args: &[
            PromptArg { name: "screen", description: "What the screen is, e.g. \"login\", \"settings\", \"order summary\"", required: true },
            PromptArg { name: "device", description: "Frame preset (default \"iPhone 16\"; see run_command frame.presets)", required: false },
            PromptArg { name: "brand_color", description: "Primary colour (default #3B5BDB)", required: false },
        ],
        template: "Design a {screen} screen for a mobile app in FigCraft.\n\n1. run_command frame.create {\"preset\": \"{device}\", \"x\": 0, \"y\": 0, \"name\": \"{screen}\"} and give it a white fill.\n2. set_properties on the frame: {\"layout\": \"vertical\", \"padding\": [64, 24, 32, 24], \"gap\": 16, \"sizingV\": \"fixed\"}.\n3. Add content with create_node using parent = the frame id: a title (text, fontSize 28, fontStyle Bold), supporting text (fontSize 16, colour #555555), inputs or list rows (frames with layout horizontal, padding 16, cornerRadius 12, stroke #E0E0E0, sizingH fill) and a primary button (frame, layout horizontal, mainAlign center, padding [14,24,14,24], cornerRadius 12, fill {brand_color}, sizingH fill, white Bold text inside).\n4. Keep spacing on an 8-point grid and text left-aligned.{verify}",
        defaults: &[("device", "iPhone 16"), ("brand_color", "#3B5BDB")],
    },
    PromptDef {
        name: "design_system",
        title: "Start a design system",
        description: "Create colour and text styles, Light/Dark colour variables, and a button component with variants.",
        args: &[
            PromptArg { name: "name", description: "Design system name (default \"Core\")", required: false },
            PromptArg { name: "primary", description: "Primary colour (default #3B5BDB)", required: false },
        ],
        template: "Set up the \"{name}\" design system in FigCraft.\n\n1. Variables: run_command variable.createCollection {\"name\": \"{name} colours\", \"modes\": [\"Light\", \"Dark\"]}, then variable.create for color/bg, color/fg and color/primary with values per mode ({\"values\": {\"Light\": …, \"Dark\": …}}); primary is {primary} in Light.\n2. Paint styles: create a swatch rectangle per colour and run style.create {\"type\": \"paint\", \"name\": \"{name}/Primary\", \"from\": <id>}; text styles likewise from text layers (Heading 32 Bold, Body 16 Regular, Caption 12).\n3. Button component: a frame with auto layout (horizontal, padding [12,20,12,20], cornerRadius 8, mainAlign center) containing a text \"Button\"; style.apply the primary paint style; component.create on it. Make a secondary variant (duplicate, outline style), then component.combineVariants on both.\n4. Bind the button fill to color/primary with variable.bind {\"ids\": [..], \"field\": \"fill\", \"variable\": <id>} and check both modes by variable.setMode on a frame.\n5. list_commands with filter \"style\" or \"variable\" shows the exact params.{verify}",
        defaults: &[("name", "Core"), ("primary", "#3B5BDB")],
    },
    PromptDef {
        name: "landing_page",
        title: "Design a landing page",
        description: "A desktop marketing page: nav, hero, feature grid, call to action and footer, all in auto layout.",
        args: &[
            PromptArg { name: "product", description: "What the page sells", required: true },
            PromptArg { name: "width", description: "Page width (default 1440)", required: false },
            PromptArg { name: "accent", description: "Accent colour (default #7048E8)", required: false },
        ],
        template: "Design a landing page for {product} in FigCraft.\n\n1. create_node a frame {\"type\": \"frame\", \"x\": 0, \"y\": 0, \"width\": {width}, \"height\": 2400, \"name\": \"Landing\", \"props\": {\"fill\": \"#FFFFFF\", \"layout\": \"vertical\", \"gap\": 0}}.\n2. Sections are child frames with sizingH fill and their own auto layout: Nav (horizontal, spaceBetween, padding [20,80,20,80]); Hero (vertical, center, padding 120, gap 24: a 64px Bold headline, an 20px subhead, a CTA button filled {accent}); Features (horizontal, gap 32, padding 80: three cards with cornerRadius 16, padding 32, a soft dropShadow effect, title + body text, each sizingH fill); CTA band filled {accent} with white text; Footer (padding 40, small grey text).\n3. Use batch to apply repeated styling to several ids in one undo step.\n4. Finish with sizingV hug on the page frame so it fits its content.{verify}",
        defaults: &[("width", "1440"), ("accent", "#7048E8")],
    },
    PromptDef {
        name: "prototype_flow",
        title: "Wire a prototype flow",
        description: "Connect screens with click interactions, add a flow starting point and check it.",
        args: &[
            PromptArg {
                name: "screens",
                description: "Comma-separated screen names, in flow order (default \"Home, Details, Checkout\")",
                required: false,
            },
            PromptArg {
                name: "transition",
                description: "Transition: instant|dissolve|smartAnimate|moveIn|push|slideIn (default smartAnimate)",
                required: false,
            },
        ],
        template: "Build a clickable prototype in FigCraft with these screens, in order: {screens}.\n\n1. If the frames don't exist, create one phone frame per screen (run_command frame.create {\"preset\": \"iPhone 16\", …}) side by side 100 units apart, each with a title and a \"Next\" button (a frame with text inside).\n2. For each screen except the last, add an interaction on its button: run_command proto.interaction.add {\"id\": <button>, \"trigger\": {\"type\": \"onClick\"}, \"action\": {\"type\": \"navigate\", \"dest\": <next frame>}, \"transition\": {\"type\": \"{transition}\"}, \"durationMs\": 300}. Add a back button on later screens with {\"action\": {\"type\": \"back\"}}.\n3. run_command proto.flow.add {\"start\": <first frame>, \"name\": \"Main flow\"}.\n4. Check get_node on each button shows `interactions`, and inspect_document shows the page's `flows`.{verify}",
        defaults: &[("screens", "Home, Details, Checkout"), ("transition", "smartAnimate")],
    },
    PromptDef {
        name: "fix_comments",
        title: "Fix comments",
        description: "Read the open comment threads and fix each one, replying and resolving it after verifying.",
        args: &[
            PromptArg { name: "page", description: "Page to read: \"all\" or a page id (default \"all\")", required: false },
            PromptArg { name: "auto_resolve", description: "yes/no: resolve threads after verifying (default yes)", required: false },
            PromptArg { name: "author", description: "Name replies are signed with (default \"AI\")", required: false },
        ],
        template: r#"Fix the open comment threads in this FigCraft document.

1. Read them: run_command {"command": "comment.list", "params": {"page": "{page}", "filter": "open"}}. Each thread has an id, text, pageName, frameName, anchor (a node id; null if the comment is only pinned to a page point), x/y, author, replies and link.
2. For each thread, in list order:
   a. Inspect the anchor with get_node {"id": <anchor>, "depth": 2} (or get_node on frame if anchor is null), and look at the frame: screenshot {"ids": [<frame>]}.
   b. Make the smallest change that answers the comment with set_properties, create_node or run_command. Put the edits for one thread in one batch so the user can undo them in one step. Do not edit layers the comment does not mention.
   c. Reply before resolving: run_command {"command": "comment.reply", "params": {"id": <id>, "text": "what you changed", "author": "{author}"}}. A reply reopens the thread, so resolving before replying leaves it open again.
   d. Verify the result with get_node/inspect_document and a screenshot. Only if the comment is actually fixed, resolve it: run_command {"command": "comment.resolve", "params": {"id": <id>}}. Auto-resolve is "{auto_resolve}": if that is "no", leave every thread open for the user and only reply.
   e. If the comment is ambiguous or the fix is not possible, reply with the question or the reason and leave the thread open. Never resolve it.
3. Finish with a screenshot of the changed area and a short report: fixed thread ids, still-open ids and why. Do not touch layers that have no comment."#,
        defaults: &[("page", "all"), ("auto_resolve", "yes"), ("author", "AI")],
    },
    PromptDef {
        name: "watch_comments",
        title: "Watch comments",
        description: "Watch the open comments and fix new feedback as it arrives.",
        args: &[PromptArg { name: "minutes", description: "How long to watch (default 5)", required: false }],
        template: r#"Watch for new feedback in this FigCraft document for {minutes} minutes.

1. Start the watcher: wait_for_changes {"after": 0, "timeoutMs": 0, "topics": ["comments"]}. Keep its `cursor` and note `reset`; if the returned cursor is 0, remember that the initial cursor was zero.
2. Handle what is already open: run_command {"command": "comment.list", "params": {"page": "all", "filter": "open"}}. For each thread: inspect the anchor and frame, make the smallest change (one batch per thread), comment.reply with what changed (it is signed with your own agent name), verify, then comment.resolve. Leave ambiguous threads open with a question.
3. Then loop until the budget ends: wait_for_changes {"after": <cursor>, "timeoutMs": 20000, "topics": ["comments"]}. Set cursor to the result's cursor. For each comment.added or comment.replied event, read the thread with comment.list. If its latest message is your own reply, skip it (comment.reply emits comment.replied too); otherwise fix it the same way. If reset is true, dropped is greater than 0, or the cursor was 0 before this poll and the returned cursor is greater than 0, re-read every open thread and continue with the returned cursor.
4. Stop after {minutes} minutes; summarize the threads you fixed and the ones still open (and why)."#,
        defaults: &[("minutes", "5")],
    },
];

pub fn list() -> Value {
    json!({"prompts": PROMPTS.iter().map(|p| json!({
        "name": p.name,
        "title": p.title,
        "description": p.description,
        "arguments": p.args.iter().map(|a| json!({"name": a.name, "description": a.description, "required": a.required})).collect::<Vec<_>>(),
    })).collect::<Vec<_>>()})
}

pub fn get(name: &str, args: &Value) -> Result<Value, String> {
    let p = PROMPTS
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| format!("unknown prompt `{name}` (have: {})", PROMPTS.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")))?;
    if !(args.is_object() || args.is_null()) {
        return Err("`arguments` must be an object of strings".into());
    }
    let mut text = p.template.replace("{verify}", VERIFY);
    for a in p.args {
        let v = match args.get(a.name) {
            Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
            Some(Value::Number(n)) => n.to_string(),
            Some(Value::Null) | None => match p.defaults.iter().find(|(k, _)| *k == a.name) {
                Some((_, d)) => (*d).to_string(),
                None if a.required => return Err(format!("prompt `{name}` needs argument `{}`", a.name)),
                None => String::new(),
            },
            Some(Value::String(_)) if a.required => return Err(format!("prompt `{name}` needs argument `{}`", a.name)),
            Some(Value::String(_)) => p.defaults.iter().find(|(k, _)| *k == a.name).map(|(_, d)| (*d).to_string()).unwrap_or_default(),
            Some(_) => return Err(format!("argument `{}` must be a string", a.name)),
        };
        text = text.replace(&format!("{{{}}}", a.name), &v);
    }
    Ok(json!({"description": p.description, "messages": [{"role": "user", "content": {"type": "text", "text": text}}]}))
}
