# TeX Primitive Checklist

This checklist tracks the original TeX82 primitive set, grouped by subsystem. Engine extensions from e-TeX, pdfTeX, XeTeX, LuaTeX, and other descendants should be tracked separately.

Use `[ ]` for not implemented, `[x]` for implemented, and add local notes after the description when behavior is partial or deliberately differs.

## Boxes And Rules

- [x] `\badness` - Reports the badness of the glue setting in the last box made. Implemented as a read-only internal integer backed by execution-side packing records.
- [x] `\box` - Appends a box register's contents and clears that register.
- [x] `\boxmaxdepth` - Sets the maximum depth allowed when building vertical boxes. Implemented as an assignable dimension parameter consumed by `tex-typeset` vertical packing.
- [x] `\cleaders` - Builds centered leaders across glue. Payload parsing and glue-node storage are implemented; DVI repetition is tracked in the leaders output issue.
- [x] `\copy` - Appends a copy of a box register without clearing it.
- [x] `\dp` - Reads or assigns the depth of a box register.
- [x] `\everyhbox` - Token list inserted at the start of every `\hbox`. Implemented as an assignable token list parameter; insertion at box start is not yet wired and is tracked in the conformance epic.
- [x] `\everyvbox` - Token list inserted at the start of every `\vbox`. Implemented as an assignable token list parameter; insertion at box start is not yet wired and is tracked in the conformance epic.
- [x] `\hbadness` - Threshold above which underfull or loose hboxes are reported. Implemented as an assignable integer parameter consumed by `tex-typeset` horizontal packing diagnostics.
- [x] `\hbox` - Builds a horizontal box.
- [x] `\hfuzz` - Tolerance before overfull hboxes are reported. Implemented as an assignable dimension parameter consumed by `tex-typeset` horizontal packing diagnostics.
- [x] `\hrule` - Adds a horizontal rule in vertical mode with TeX's running-width/default-thickness rule dimensions and resets `\prevdepth` to the ignore sentinel.
- [x] `\ht` - Reads or assigns the height of a box register.
- [x] `\lastbox` - Removes and returns the last box from the current list when allowed.
- [x] `\leaders` - Repeats a box or rule across glue. Payload parsing and glue-node storage are implemented; DVI repetition is tracked in the leaders output issue.
- [x] `\overfullrule` - Width of the diagnostic rule added to overfull boxes. Implemented as an assignable dimension parameter; overfull hboxes get the diagnostic rule appended during packing.
- [x] `\prevdepth` - Depth of the previous box on the current vertical list. Implemented as a per-mode-list field; `\nointerlineskip` sets TeX's ignore sentinel.
- [x] `\setbox` - Assigns a box register from an `\hbox`, `\vbox`, or `\vtop`.
- [x] `\unhbox` - Unpacks an hbox register into the current list and clears it.
- [x] `\unhcopy` - Unpacks a copy of an hbox register into the current list.
- [x] `\unvbox` - Unpacks a vbox register into the current list and clears it; vertical children are appended through the shared interline-glue path.
- [x] `\unvcopy` - Unpacks a copy of a vbox register into the current list.
- [x] `\vbadness` - Threshold above which underfull or loose vboxes are reported. Implemented as an assignable integer parameter consumed by `tex-typeset` vertical packing diagnostics.
- [x] `\vbox` - Builds a vertical box with normal baseline positioning.
- [x] `\vfuzz` - Tolerance before overfull vboxes are reported. Implemented as an assignable dimension parameter consumed by `tex-typeset` vertical packing diagnostics.
- [x] `\vrule` - Adds a vertical rule in horizontal mode.
- [x] `\vtop` - Builds a vertical box aligned by its first item.
- [x] `\wd` - Reads or assigns the width of a box register.
- [x] `\xleaders` - Builds expanded leaders across glue. Payload parsing and glue-node storage are implemented; DVI repetition is tracked in the leaders output issue.

## Characters And Case

- [x] `\ ` - Inserts an explicit control space as normal current-font interword glue with space factor 1000.
- [x] `\accent` - Places a text accent over the following character.
- [x] `\catcode` - Reads or assigns a character's category code; assignments use the code-table facade and bump generations.
- [x] `\char` - Produces a character token by numeric character code.
- [x] `\chardef` - Defines a control sequence as a character-code command usable as an internal integer.
- [x] `\endlinechar` - Character appended when TeX tokenizes an input line. Implemented as an assignable integer parameter for lexing and value rendering.
- [x] `\escapechar` - Character used when printing control sequence names. Implemented as an assignable integer parameter for value-rendering expandables.
- [x] `\lccode` - Reads or assigns a character's lowercase mapping; assignments use the code-table facade and bump generations.
- [x] `\lowercase` - Converts character tokens using `\lccode`.
- [x] `\newlinechar` - Character that starts a new line in terminal, log, and stream output. Implemented as an assignable integer parameter honored when rendering `\message`, `\errmessage`, and `\write` text.
- [x] `\number` - Expands an integer as decimal character tokens using the shared expanded integer scanner.
- [x] `\romannumeral` - Expands an integer as lowercase roman numeral tokens; non-positive values expand to an empty frozen token list.
- [x] `\sfcode` - Reads or assigns a character's space-factor code; assignments use the code-table facade and bump generations.
- [x] `\string` - Expands a token into its character representation with `\escapechar` handling and frozen output token lists.
- [x] `\uccode` - Reads or assigns a character's uppercase mapping; assignments use the code-table facade and bump generations.
- [x] `\uppercase` - Converts character tokens using `\uccode`.

## Diagnostics And Interaction

- [ ] `\batchmode` - Suppresses terminal interaction and most terminal output.
- [x] `\errhelp` - Token list shown as help for a following `\errmessage`. Implemented as an assignable token list parameter; help display is tracked with the conformance error-format pass.
- [x] `\errmessage` - Issues an error with expanded message text through World's terminal/log effect sink; interactive help/context remains World/interaction work.
- [x] `\errorcontextlines` - Number of context lines shown for errors. Implemented as an assignable integer parameter; error-context display is tracked with the conformance error-format pass.
- [ ] `\errorstopmode` - Restores interactive stopping on errors.
- [x] `\meaning` - Expands to a textual description of a token's meaning. Macro text is supported; unsupported raw meanings use a placeholder.
- [x] `\message` - Writes expanded message text through World's terminal/log effect sink with pdfTeX-style message separation and wrapping for the covered subset.
- [ ] `\nonstopmode` - Continues past errors without stopping for input.
- [x] `\pausing` - Prompts after input lines when positive. Implemented as an assignable integer parameter; prompting is intentionally not implemented in the batch-first engine.
- [ ] `\scrollmode` - Scrolls past errors while still showing diagnostics.
- [x] `\show` - Displays the meaning of the next token through World's terminal/log effect sink for implemented meaning classes.
- [x] `\showbox` - Writes a box register's contents to the log.
- [x] `\showboxbreadth` - Maximum number of list items shown per level. Implemented as an assignable integer parameter; the `\showbox` emitter is tracked separately.
- [x] `\showboxdepth` - Maximum nesting depth shown for box diagnostics. Implemented as an assignable integer parameter; the `\showbox` emitter is tracked separately.
- [x] `\showlists` - Writes the current mostly-empty mode nest in pdfTeX format through World's terminal/log effect sink.
- [x] `\showhyphens` - Displays current automatic hyphenation points using loaded patterns and exceptions for language 0.
- [x] `\showthe` - Displays the value produced by implemented `\the` targets through World's terminal/log effect sink.
- [x] `\tracingcommands` - Logs command execution when positive. Assignable integer parameter; trace emission is tracked in the conformance epic.
- [x] `\tracinglostchars` - Logs missing font characters when positive. Assignable integer parameter; trace emission is tracked in the conformance epic.
- [x] `\tracingmacros` - Logs macro expansion and arguments when positive. Assignable integer parameter; trace emission is tracked in the conformance epic.
- [x] `\tracingonline` - Mirrors diagnostic log output to the terminal when positive. Assignable integer parameter; trace emission is tracked in the conformance epic.
- [x] `\tracingoutput` - Logs shipped-out box contents when positive. Assignable integer parameter; trace emission is tracked in the conformance epic.
- [x] `\tracingpages` - Logs page builder calculations when positive. Assignable integer parameter; trace emission is tracked in the conformance epic.
- [x] `\tracingparagraphs` - Logs line-breaking calculations when positive. Assignable integer parameter; trace emission is tracked in the conformance epic.
- [x] `\tracingrestores` - Logs save-stack restoration when positive. Assignable integer parameter; trace emission is tracked in the conformance epic.
- [x] `\tracingstats` - Logs memory statistics at job end when positive. Assignable integer parameter; trace emission is tracked in the conformance epic.

## File I/O And Output

- [x] `\closein` - Closes a World input stream slot.
- [x] `\closeout` - Closes a World output stream slot through a deferred whatsit fired at shipout.
- [x] `\endinput` - Stops reading the current input source after the current line through the lexer input stack; source identity is pinned by World input records and lexer summaries can restore mid-source positions.
- [x] `\immediate` - Executes the following output stream operation immediately for `\openout`, `\write`, and `\closeout`.
- [x] `\input` - Pushes a driver-provided source at the current input position. `tex-expand` scans the file name and calls a source hook; it does not read the filesystem directly.
- [ ] `\inputlineno` - Current line number in the active input file.
- [x] `\openin` - Opens a World-backed content-addressed input stream; missing files leave the slot at EOF.
- [x] `\openout` - Opens a World output stream through a deferred whatsit fired at shipout.
- [x] `\output` - Token list invoked by the page builder for output routine processing; output routines run through the shared main-control driver with `\box255` handling.
- [x] `\read` - Reads one line from an opened World input stream and defines the target control sequence as a no-parameter macro; terminal reads remain unsupported in nonstop batch execution.
- [x] `\shipout` - Writes a completed box to the DVI output. Ships to the committed page artifact model in `tex-out` and fires deferred whatsits; DVI byte emission is tracked separately.
- [x] `\special` - Emits backend-specific material into the DVI stream. Adds a whatsit node carried through shipout into the page artifact.
- [x] `\write` - Writes expanded material to an output stream, normally delayed until shipout. Appends a deferred-write whatsit fired at shipout through World effect records; the `\immediate` prefix is tracked separately.

## Fonts

- [x] `\/` - Inserts italic correction for the preceding character or ligature.
- [x] `\font` - Defines a font control sequence, loads TFM metrics via `World`, and reuses loaded font ids for the same name, selected size, and content.
- [x] `\fontdimen` - Reads or assigns Env-backed per-font parameters with grouping and TeX's most-recent-font growth rule.
- [x] `\fontname` - Expands a real font selector to its TFM name, including an `at <size>` suffix when selected size differs from design size.
- [x] `\nullfont` - Predefined empty font used when no real font is selected; installed as the initial current font in no-format runs.

## Glue And Skips

- [x] `\hfil` - Inserts first-order infinitely stretchable horizontal glue.
- [x] `\hfill` - Inserts second-order infinitely stretchable horizontal glue.
- [x] `\hfilneg` - Inserts negative first-order horizontal stretch.
- [x] `\hskip` - Inserts horizontal glue.
- [x] `\hss` - Inserts horizontal glue with infinite stretch and shrink.
- [x] `\lastskip` - Reports the last glue on the current list, preserving stretch and shrink order, or zero.
- [x] `\unskip` - Removes the last glue item from the current list when allowed.
- [x] `\vfil` - Inserts first-order infinitely stretchable vertical glue.
- [x] `\vfill` - Inserts second-order infinitely stretchable vertical glue.
- [x] `\vfilneg` - Inserts negative first-order vertical stretch.
- [x] `\vskip` - Inserts vertical glue.
- [x] `\vss` - Inserts vertical glue with infinite stretch and shrink.

## Hyphenation And Languages

- [x] `\-` - Inserts a discretionary hyphen.
- [x] `\defaulthyphenchar` - Default `\hyphenchar` value for newly loaded fonts; implemented as an integer parameter used when initializing font banks.
- [x] `\discretionary` - Adds an explicit discretionary break with pre, post, and replacement text.
- [x] `\hyphenation` - Adds lccode-normalized hyphenation exceptions for language 0; exceptions override pattern-derived positions.
- [x] `\hyphenchar` - Reads or assigns the Env-backed hyphenation character for a font selector.
- [x] `\language` - Selects the current hyphenation language. Implemented as an assignable integer parameter; patterns and exceptions exist for language 0 only, and language switching is deferred.
- [x] `\lefthyphenmin` - Minimum characters before the first automatic hyphen; consumed by hlist hyphenation and `\showhyphens`.
- [x] `\patterns` - Adds lccode-normalized INITEX-style Liang patterns for language 0 into the snapshot-covered hyphenation trie.
- [x] `\righthyphenmin` - Minimum characters after the last automatic hyphen; consumed by hlist hyphenation and `\showhyphens`.
- [ ] `\setlanguage` - Inserts a language whatsit into the current horizontal list.
- [x] `\uchyph` - Enables hyphenation of words beginning with uppercase letters when positive.

## Insertions And Splits

- [x] `\holdinginserts` - Keeps insertions on the page list during output routine processing when positive.
- [x] `\insert` - Adds internal vertical material to an insertion class, capturing split parameters and participating in page-builder insertion splitting/distribution.
- [x] `\insertpenalties` - Reports split/floating insertion penalties in mainline and held-over insertion count while an output routine is active.
- [x] `\splitbotmark` - Expands to the last mark captured by the most recent `\vsplit` or insertion split.
- [x] `\splitfirstmark` - Expands to the first mark captured by the most recent `\vsplit` or insertion split.
- [x] `\splitmaxdepth` - Maximum depth allowed in split boxes; assignable and consumed by `\vsplit` and insertion splitting.
- [x] `\splittopskip` - Top glue inserted in split boxes; assignable and consumed by `\vsplit` and insertion splitting.
- [x] `\vsplit` - Splits vertical material from a box register to a target height, setting the split marks.

## Job Control

- [x] `\day` - Job-start day of the month. Assignable integer parameter initialized from `World`'s job clock; parity regeneration pins `SOURCE_DATE_EPOCH` for both Umber and reference TeX.
- [x] `\deadcycles` - Number of output routine calls since the last `\shipout`; read/write page-builder counter.
- [ ] `\dump` - Writes a format file in INITEX; otherwise ends the job.
- [x] `\end` - Ends the job with TeX's final cleanup, flushing remaining page material through the output routine before exiting the batch loop.
- [x] `\everyjob` - Token list inserted at the start of every job. Implemented as an assignable token list parameter; job-start insertion is tracked with plain.tex bring-up in the conformance epic.
- [x] `\jobname` - Expands to the driver-provided job name as rendered character tokens.
- [x] `\mag` - Magnification ratio, scaled by 1000; implemented as an assignable integer parameter used by true-unit scanning.
- [x] `\maxdeadcycles` - Maximum allowed output routine cycles without shipout; consumed by the output-routine loop guard.
- [x] `\month` - Job-start month number. Assignable integer parameter initialized from `World`'s job clock; parity regeneration pins `SOURCE_DATE_EPOCH` for both Umber and reference TeX.
- [x] `\time` - Job-start minutes after midnight. Assignable integer parameter initialized from `World`'s job clock; parity regeneration pins `SOURCE_DATE_EPOCH` for both Umber and reference TeX.
- [x] `\year` - Job-start year. Assignable integer parameter initialized from `World`'s job clock; parity regeneration pins `SOURCE_DATE_EPOCH` for both Umber and reference TeX.

## Kerns And Box Motion

- [x] `\kern` - Adds an explicit kern to the current list.
- [x] `\lastkern` - Reports the last kern on the current list, or zero.
- [x] `\lower` - Lowers a box in horizontal or math mode.
- [x] `\moveleft` - Moves a box left in vertical mode.
- [x] `\moveright` - Moves a box right in vertical mode.
- [x] `\raise` - Raises a box in horizontal or math mode.
- [x] `\unkern` - Removes the last kern from the current list when allowed.

## Conditionals

- [x] `\else` - Starts the else branch of implemented conditionals, skipping already-taken limbs through the real token stream with nested conditional tracking and extra-control diagnostics.
- [x] `\fi` - Ends an implemented conditional and reports an extra-control diagnostic when no conditional is open.
- [x] `\if` - Compares two expanded unexpandable character tokens by character code.
- [x] `\ifcase` - Selects among numeric conditional branches using the shared integer scanner, `\or` counting, else fallback, and token-level skipped-limb scanning.
- [x] `\ifcat` - Compares two expanded unexpandable tokens by category code.
- [x] `\ifdim` - Compares two dimensions using the shared dimension scanner.
- [x] `\ifeof` - Tests World input stream state; unopened or exhausted streams are EOF.
- [x] `\iffalse` - Starts a conditional that is always false.
- [x] `\ifhbox` - Tests readable box register state for an hlist box.
- [x] `\ifhmode` - Tests the driver-supplied mode query.
- [x] `\ifinner` - Tests the driver-supplied inner-mode query.
- [x] `\ifmmode` - Tests the driver-supplied mode query.
- [x] `\ifnum` - Compares two integers using the shared integer scanner.
- [x] `\ifodd` - Tests whether an integer scanned by the shared integer scanner is odd.
- [x] `\iftrue` - Starts a conditional that is always true.
- [x] `\ifvbox` - Tests readable box register state for a vlist box.
- [x] `\ifvmode` - Tests the driver-supplied mode query; the no-driver default is outer vertical mode.
- [x] `\ifvoid` - Tests readable box register state for TeX's void box.
- [x] `\ifx` - Compares two unexpanded tokens by meaning, including hash-consed macro definition identity.
- [x] `\or` - Separates implemented `\ifcase` branches, advances the selected limb count, skips remaining taken branches, and reports extra-control diagnostics outside `\ifcase`.

## Macros, Expansion, And Grouping

- [x] `\afterassignment` - Saves a token in snapshot-covered state and inserts it after the next completed assignment, including box assignments.
- [x] `\aftergroup` - Saves a token on the current state-layer group marker and replays saved tokens FIFO when that group exits.
- [x] `\begingroup` - Starts an explicit semi-simple group through the state journal marker API.
- [x] `\csname` - Builds a control sequence from expanded character tokens and assigns `\relax` to newly-created undefined names through the explicit expansion interning capability.
- [x] `\def` - Defines a macro without expanding replacement text.
- [x] `\edef` - Defines a macro after expanding replacement text.
- [x] `\endcsname` - Terminates a `\csname` name scan.
- [x] `\endgroup` - Ends an explicit semi-simple group through the state journal marker API with boundary-kind mismatch diagnostics.
- [x] `\expandafter` - Expands the token after the next token before continuing.
- [x] `\futurelet` - Assigns a control sequence to the following token while preserving input.
- [x] `\gdef` - Globally defines a macro without expanding replacement text.
- [x] `\global` - Prefix that makes the following assignment global.
- [x] `\globaldefs` - Overrides local or global assignment behavior by sign.
- [x] `\let` - Gives a control sequence the current meaning of another token.
- [x] `\long` - Prefix allowing a macro parameter to contain `\par`.
- [x] `\noexpand` - Suppresses expansion of the next token during expansion-only contexts.
- [x] `\outer` - Prefix marking a macro invalid in restricted scanning contexts.
- [x] `\protected` - Prefix marking a macro as protected from expansion.
- [x] `\relax` - No-op command that can terminate scans or absorb expansion.
- [x] `\the` - Expands supported internal quantities or token register values. Current support covers integer, dimension, glue, muglue, and token registers; register aliases; Env-backed parameters; and code-table values.
- [x] `\xdef` - Globally defines a macro after expanding replacement text.

## Marks

- [x] `\botmark` - Expands to the last mark on the current page, updated by the page builder at output-routine fire-up.
- [x] `\firstmark` - Expands to the first mark on the current page, updated by the page builder at output-routine fire-up.
- [x] `\mark` - Adds frozen mark text to the current list; the page builder captures marks at fire-up and splits.
- [x] `\topmark` - Expands to the `\botmark` value from the preceding page, maintained by the page builder at output-routine fire-up.

## Math

- [x] `\above` - Builds a generalized fraction with explicit rule thickness and no delimiters; Appendix G layout is implemented by the math kernel.
- [x] `\abovedisplayshortskip` - Glue above a display when the preceding line is short.
- [x] `\abovedisplayskip` - Normal glue above a display.
- [x] `\abovewithdelims` - Builds a generalized fraction with delimiters and explicit rule thickness; Appendix G delimiter sizing/layout is implemented by the math kernel.
- [x] `\atop` - Builds a fraction-like stack with no rule and no delimiters; Appendix G layout is implemented by the math kernel.
- [x] `\atopwithdelims` - Builds a fraction-like stack with delimiters and no rule; Appendix G delimiter sizing/layout is implemented by the math kernel.
- [x] `\belowdisplayshortskip` - Glue below a display when short-display spacing is used.
- [x] `\belowdisplayskip` - Normal glue below a display.
- [x] `\binoppenalty` - Penalty inserted after binary operators when inline math is converted for paragraph line breaking.
- [x] `\defaultskewchar` - Default `\skewchar` value for newly loaded fonts; implemented as an integer parameter used when initializing font banks.
- [x] `\delcode` - Reads or assigns a character's delimiter code; assignments use the code-table facade and bump generations.
- [x] `\delimiter` - Adds a delimiter by numeric delimiter code; Appendix G variable delimiter sizing is implemented by the math kernel.
- [x] `\delimiterfactor` - Scaling factor used to choose `\left...\right` delimiter sizes.
- [x] `\delimitershortfall` - Allowed shortfall when choosing `\left...\right` delimiter sizes.
- [x] `\displayindent` - Indentation applied to the current display.
- [x] `\displaylimits` - Uses default limit placement for large operators, including display-style limits above and below.
- [x] `\displaystyle` - Selects display math style during Appendix G conversion.
- [x] `\displaywidowpenalty` - Penalty before a display after the penultimate paragraph line.
- [x] `\displaywidth` - Line width available to the current display.
- [x] `\eqno` - Adds a right-side equation number to a display.
- [x] `\everydisplay` - Token list inserted when entering display math.
- [x] `\everymath` - Token list inserted when entering inline math.
- [x] `\fam` - Current math family for variable-family math characters.
- [x] `\left` - Starts a delimited math subformula with scalable left delimiter.
- [x] `\leqno` - Adds a left-side equation number to a display.
- [x] `\limits` - Forces limits above and below a large operator.
- [x] `\mathaccent` - Adds a math accent atom with Appendix G accent placement and skewchar kerning.
- [x] `\mathbin` - Treats the following item as a binary operator atom with Appendix G spacing and penalties.
- [x] `\mathchar` - Adds a math character by numeric math code.
- [x] `\mathchardef` - Defines a control sequence as a math character command usable as an internal integer.
- [x] `\mathchoice` - Provides alternatives for display, text, script, and scriptscript styles; Appendix G selects the active arm during conversion.
- [x] `\mathclose` - Treats the following item as a closing atom with Appendix G spacing.
- [x] `\mathcode` - Reads or assigns a character's math code; assignments use the code-table facade and bump generations.
- [x] `\mathinner` - Treats the following subformula as an inner atom with Appendix G spacing.
- [x] `\mathop` - Treats the following item as a large-operator atom with limit placement, larger display variants, and axis centering.
- [x] `\mathopen` - Treats the following item as an opening atom with Appendix G spacing.
- [x] `\mathord` - Treats the following item as an ordinary atom with Appendix G spacing and adjacent-symbol ligature/kern handling.
- [x] `\mathpunct` - Treats the following item as a punctuation atom with Appendix G spacing.
- [x] `\mathrel` - Treats the following item as a relation atom with Appendix G spacing and penalties.
- [x] `\mathsurround` - Extra space inserted around inline math through math-on/math-off nodes.
- [x] `\medmuskip` - Medium math glue between math atoms.
- [x] `\mkern` - Adds a math kern converted from mu units during Appendix G conversion.
- [x] `\mskip` - Adds math glue converted from mu units during Appendix G conversion.
- [x] `\muskip` - Reads or assigns a math glue register, including sparse e-TeX indices.
- [x] `\muskipdef` - Defines a symbolic name for a math glue register.
- [x] `\nolimits` - Forces limits to the side of a large operator.
- [x] `\nonscript` - Suppresses following glue or kern in script and smaller styles during Appendix G conversion.
- [x] `\nulldelimiterspace` - Width reserved for missing delimiters.
- [x] `\over` - Builds a normal fraction with no delimiters; Appendix G layout is implemented by the math kernel.
- [x] `\overline` - Places a rule over the following math item with Appendix G clearance.
- [x] `\overwithdelims` - Builds a normal fraction with delimiters; Appendix G delimiter sizing/layout is implemented by the math kernel.
- [x] `\postdisplaypenalty` - Penalty inserted after a display.
- [x] `\predisplaypenalty` - Penalty inserted before a display.
- [x] `\predisplaysize` - Effective width of the line preceding a display.
- [x] `\radical` - Builds a radical atom from a delimiter code and nucleus with Appendix G clearance and extensible radical sizing.
- [x] `\relpenalty` - Penalty inserted after relation atoms when inline math is converted for paragraph line breaking.
- [x] `\right` - Ends a delimited math subformula with scalable right delimiter.
- [x] `\scriptfont` - Font used for a family in script style during Appendix G glyph selection.
- [x] `\scriptscriptfont` - Font used for a family in scriptscript style during Appendix G glyph selection.
- [x] `\scriptscriptstyle` - Selects scriptscript math style during Appendix G conversion.
- [x] `\scriptspace` - Extra space after subscripts and superscripts.
- [x] `\scriptstyle` - Selects script math style during Appendix G conversion.
- [x] `\skewchar` - Font-specific Env-backed character used to position math accents.
- [x] `\textfont` - Font used for a family in text style during Appendix G glyph selection.
- [x] `\textstyle` - Selects text math style during Appendix G conversion.
- [x] `\thickmuskip` - Thick math glue between math atoms.
- [x] `\thinmuskip` - Thin math glue between math atoms.
- [x] `\underline` - Places a rule under the following math item with Appendix G clearance.
- [x] `\vcenter` - Builds a vertically centered box for math formulas and centers it on the math axis during Appendix G conversion.

## Page Builder

- [x] `\hoffset` - Horizontal offset added to the default one-inch origin. Assignable dimension parameter; origin application belongs to DVI byte emission and is tracked in the conformance epic.
- [x] `\maxdepth` - Maximum depth allowed on the main vertical page; consumed by the page builder's depth clamp, moving excess depth into `\pagetotal`.
- [x] `\pagedepth` - Depth of the last box on the current page; read/write page-builder accumulator.
- [x] `\pagefilllstretch` - Third-order infinite stretch currently on the page; read/write page-builder accumulator.
- [x] `\pagefillstretch` - Second-order infinite stretch currently on the page; read/write page-builder accumulator.
- [x] `\pagefilstretch` - First-order infinite stretch currently on the page; read/write page-builder accumulator.
- [x] `\pagegoal` - Target height for the current page; read/write page-builder accumulator initialized from `\vsize`.
- [x] `\pageshrink` - Finite shrink currently on the page; read/write page-builder accumulator.
- [x] `\pagestretch` - Finite stretch currently on the page; read/write page-builder accumulator.
- [x] `\pagetotal` - Natural height accumulated on the current page; read/write page-builder accumulator.
- [x] `\topskip` - Glue inserted before the first box on a page; consumed by the page builder.
- [x] `\voffset` - Vertical offset added to the default one-inch origin. Assignable dimension parameter; origin application belongs to DVI byte emission and is tracked in the conformance epic.
- [x] `\vsize` - Target page body height; initializes `\pagegoal` when the first box reaches the page.

## Paragraphs And Line Breaking

- [x] `\adjdemerits` - Demerits for adjacent visually incompatible lines; consumed by the paragraph line breaker.
- [x] `\baselineskip` - Preferred glue between adjacent baselines; consumed by the shared vertical append routine.
- [x] `\doublehyphendemerits` - Demerits for consecutive hyphenated lines; consumed by the paragraph line breaker.
- [x] `\emergencystretch` - Extra stretch used during the final paragraph line-breaking pass.
- [x] `\everypar` - Token list inserted at the start of each paragraph and replayed through the input stack.
- [x] `\finalhyphendemerits` - Captured for paragraph breaking; full penultimate-line parity is tracked with the remaining line-break parity follow-up.
- [x] `\hangafter` - Line number where hanging indentation changes. Captured at `\par` and reset after paragraph completion.
- [x] `\hangindent` - Hanging indentation amount for paragraphs. Captured at `\par` and reset after paragraph completion.
- [x] `\hsize` - Line width for normal paragraph building; captured at `\par` and used to hpack each broken line.
- [x] `\ignorespaces` - Skips following space tokens and replays the first nonspace token.
- [x] `\indent` - Starts an indented paragraph.
- [x] `\leftskip` - Glue added to the left of every line; captured for the paragraph handoff.
- [x] `\lineskip` - Fallback interline glue when baseline glue would be too small; consumed by the shared vertical append routine.
- [x] `\lineskiplimit` - Threshold for using `\lineskip` instead of `\baselineskip`.
- [x] `\looseness` - Requests more or fewer lines than the optimal paragraph; captured for the paragraph handoff.
- [x] `\noboundary` - Suppresses ligature and kern boundary processing.
- [x] `\noindent` - Starts an unindented paragraph.
- [x] `\par` - Ends the current paragraph, captures paragraph parameters, runs the pure line breaker, then appends hpacked lines to the enclosing vertical list.
- [x] `\parfillskip` - Glue appended to the final line of a paragraph before the paragraph handoff.
- [x] `\parindent` - Width of paragraph indentation.
- [x] `\parshape` - Defines per-line indentation and width; stored per nest level and captured at `\par`.
- [x] `\parskip` - Glue inserted by the enclosing vertical list before a paragraph starts.
- [x] `\pretolerance` - Badness threshold for the no-hyphenation line-breaking pass; negative values skip the first pass.
- [x] `\prevgraf` - Number of lines in the most recent paragraph contribution; read/write through the enclosing vertical mode level.
- [x] `\rightskip` - Glue added to the right of every line; captured for the paragraph handoff.
- [x] `\spacefactor` - Current space factor used for interword spacing.
- [x] `\spaceskip` - Explicit interword glue override.
- [x] `\tolerance` - Badness threshold for line breaking with hyphenation; consumed by the second and emergency passes.
- [x] `\vadjust` - Inserts internal vertical material associated with the current paragraph line and migrates it after line breaking.
- [x] `\xspaceskip` - Explicit intersentence glue override.

## Penalties

- [x] `\brokenpenalty` - Penalty after a hyphenated paragraph line; inserted by post-line-break surgery.
- [x] `\clubpenalty` - Penalty after the first line of a paragraph; inserted by post-line-break surgery.
- [x] `\exhyphenpenalty` - Penalty for line breaks after explicit hyphens; consumed by the paragraph line breaker.
- [x] `\floatingpenalty` - Penalty for insertions floating after their class has split on the current page.
- [x] `\hyphenpenalty` - Penalty for line breaks at discretionary hyphens; consumed by the paragraph line breaker.
- [x] `\interlinepenalty` - Penalty inserted between paragraph lines by post-line-break surgery.
- [x] `\lastpenalty` - Reports the last penalty on the current list, or zero.
- [x] `\linepenalty` - Base demerit contribution for each broken line; consumed by the paragraph line breaker.
- [x] `\outputpenalty` - Penalty value that triggered the current output routine; set globally by the page builder at the chosen break and assignable in output routines.
- [x] `\penalty` - Adds a penalty node to the current list.
- [x] `\unpenalty` - Removes the last penalty from the current list when allowed.
- [x] `\widowpenalty` - Penalty after the penultimate line of a paragraph; inserted by post-line-break surgery.

## Registers And Arithmetic

- [x] `\advance` - Adds to an integer, dimension, glue, or muglue quantity with TeX-style overflow diagnostics.
- [x] `\count` - Reads or assigns an integer register, including sparse e-TeX indices.
- [x] `\countdef` - Defines a symbolic name for an integer register.
- [x] `\dimen` - Reads or assigns a dimension register, including sparse e-TeX indices.
- [x] `\dimendef` - Defines a symbolic name for a dimension register.
- [x] `\divide` - Divides an integer, dimension, glue, or muglue quantity by an integer with TeX-style overflow diagnostics.
- [x] `\multiply` - Multiplies an integer, dimension, glue, or muglue quantity by an integer with TeX-style overflow diagnostics.
- [x] `\skip` - Reads or assigns a glue register, including sparse e-TeX indices.
- [x] `\skipdef` - Defines a symbolic name for a glue register.
- [x] `\toks` - Reads or assigns a token-list register, including sparse e-TeX indices and balanced text assignment.
- [x] `\toksdef` - Defines a symbolic name for a token-list register.

## Alignments

- [x] `\cr` - Ends an implemented alignment cell/row in the unset-record sub-mode; final width resolution is pending.
- [x] `\crcr` - Ends or skips alignment rows in the implemented unset-record sub-mode; `\everycr` is pending.
- [ ] `\everycr` - Token list inserted after `\cr` or nonredundant `\crcr`.
- [x] `\halign` - Parses preambles and executes rows/cells into unset records; two-pass width resolution is pending.
- [ ] `\noalign` - Inserts vertical material between alignment rows.
- [x] `\omit` - Ignores the current alignment entry template in the implemented alignment sub-mode.
- [x] `\span` - Combines adjacent alignment columns in the implemented unset-record sub-mode; final span width distribution is pending.
- [x] `\tabskip` - Glue captured from preambles and inserted between unset alignment cells/rows.
- [x] `\valign` - Parses preambles and executes rows/cells into unset records; transposition and final width resolution are pending.
