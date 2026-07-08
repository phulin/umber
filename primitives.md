# TeX Primitive Checklist

This checklist tracks the original TeX82 primitive set, grouped by subsystem. Engine extensions from e-TeX, pdfTeX, XeTeX, LuaTeX, and other descendants should be tracked separately.

Use `[ ]` for not implemented, `[x]` for implemented, and add local notes after the description when behavior is partial or deliberately differs.

## Boxes And Rules

- [ ] `\badness` - Reports the badness of the glue setting in the last box made.
- [ ] `\box` - Appends a box register's contents and clears that register.
- [x] `\boxmaxdepth` - Sets the maximum depth allowed when building vertical boxes. Implemented as an assignable dimension parameter consumed by `tex-typeset` vertical packing.
- [ ] `\cleaders` - Builds centered leaders across glue.
- [ ] `\copy` - Appends a copy of a box register without clearing it.
- [ ] `\dp` - Reads or assigns the depth of a box register.
- [ ] `\everyhbox` - Token list inserted at the start of every `\hbox`.
- [ ] `\everyvbox` - Token list inserted at the start of every `\vbox`.
- [x] `\hbadness` - Threshold above which underfull or loose hboxes are reported. Implemented as an assignable integer parameter consumed by `tex-typeset` horizontal packing diagnostics.
- [ ] `\hbox` - Builds a horizontal box.
- [x] `\hfuzz` - Tolerance before overfull hboxes are reported. Implemented as an assignable dimension parameter consumed by `tex-typeset` horizontal packing diagnostics.
- [x] `\hrule` - Adds a horizontal rule in vertical mode with TeX's running-width/default-thickness rule dimensions and resets `\prevdepth` to the ignore sentinel.
- [ ] `\ht` - Reads or assigns the height of a box register.
- [ ] `\lastbox` - Removes and returns the last box from the current list when allowed.
- [ ] `\leaders` - Repeats a box or rule across glue.
- [x] `\overfullrule` - Width of the diagnostic rule added to overfull boxes. Implemented as an assignable dimension parameter read by `tex-typeset`; diagnostic rule insertion remains tied to future box primitive emission.
- [x] `\prevdepth` - Depth of the previous box on the current vertical list. Implemented as a per-mode-list field; `\nointerlineskip` sets TeX's ignore sentinel.
- [x] `\setbox` - Assigns a box register from an `\hbox`, `\vbox`, or `\vtop`.
- [x] `\unhbox` - Unpacks an hbox register into the current list and clears it.
- [ ] `\unhcopy` - Unpacks a copy of an hbox register into the current list.
- [x] `\unvbox` - Unpacks a vbox register into the current list and clears it; vertical children are appended through the shared interline-glue path.
- [ ] `\unvcopy` - Unpacks a copy of a vbox register into the current list.
- [x] `\vbadness` - Threshold above which underfull or loose vboxes are reported. Implemented as an assignable integer parameter consumed by `tex-typeset` vertical packing diagnostics.
- [x] `\vbox` - Builds a vertical box with normal baseline positioning.
- [x] `\vfuzz` - Tolerance before overfull vboxes are reported. Implemented as an assignable dimension parameter consumed by `tex-typeset` vertical packing diagnostics.
- [x] `\vrule` - Adds a vertical rule in horizontal mode.
- [x] `\vtop` - Builds a vertical box aligned by its first item.
- [x] `\wd` - Reads or assigns the width of a box register.
- [ ] `\xleaders` - Builds expanded leaders across glue.

## Characters And Case

- [ ] `\ ` - Inserts an explicit control space.
- [x] `\accent` - Places a text accent over the following character.
- [x] `\catcode` - Reads or assigns a character's category code; assignments use the code-table facade and bump generations.
- [x] `\char` - Produces a character token by numeric character code.
- [x] `\chardef` - Defines a control sequence as a character-code command usable as an internal integer.
- [x] `\endlinechar` - Character appended when TeX tokenizes an input line. Implemented as an assignable integer parameter for lexing and value rendering.
- [x] `\escapechar` - Character used when printing control sequence names. Implemented as an assignable integer parameter for value-rendering expandables.
- [x] `\lccode` - Reads or assigns a character's lowercase mapping; assignments use the code-table facade and bump generations.
- [x] `\lowercase` - Converts character tokens using `\lccode`.
- [ ] `\newlinechar` - Character that starts a new line in terminal or log output.
- [x] `\number` - Expands an integer as decimal character tokens using the shared expanded integer scanner.
- [x] `\romannumeral` - Expands an integer as lowercase roman numeral tokens; non-positive values expand to an empty frozen token list.
- [x] `\sfcode` - Reads or assigns a character's space-factor code; assignments use the code-table facade and bump generations.
- [x] `\string` - Expands a token into its character representation with `\escapechar` handling and frozen output token lists.
- [x] `\uccode` - Reads or assigns a character's uppercase mapping; assignments use the code-table facade and bump generations.
- [x] `\uppercase` - Converts character tokens using `\uccode`.

## Diagnostics And Interaction

- [ ] `\batchmode` - Suppresses terminal interaction and most terminal output.
- [ ] `\errhelp` - Token list shown as help for a following `\errmessage`.
- [x] `\errmessage` - Issues an error with expanded message text through World's terminal/log effect sink; interactive help/context remains World/interaction work.
- [ ] `\errorcontextlines` - Number of context lines shown for errors.
- [ ] `\errorstopmode` - Restores interactive stopping on errors.
- [x] `\meaning` - Expands to a textual description of a token's meaning. Macro text is supported; unsupported raw meanings use a placeholder.
- [x] `\message` - Writes expanded message text through World's terminal/log effect sink with pdfTeX-style message separation and wrapping for the covered subset.
- [ ] `\nonstopmode` - Continues past errors without stopping for input.
- [ ] `\pausing` - Prompts after input lines when positive.
- [ ] `\scrollmode` - Scrolls past errors while still showing diagnostics.
- [x] `\show` - Displays the meaning of the next token through World's terminal/log effect sink for implemented meaning classes.
- [x] `\showbox` - Writes a box register's contents to the log.
- [x] `\showboxbreadth` - Maximum number of list items shown per level. Implemented as an assignable integer parameter; the `\showbox` emitter is tracked separately.
- [x] `\showboxdepth` - Maximum nesting depth shown for box diagnostics. Implemented as an assignable integer parameter; the `\showbox` emitter is tracked separately.
- [x] `\showlists` - Writes the current mostly-empty mode nest in pdfTeX format through World's terminal/log effect sink.
- [x] `\showhyphens` - Displays current automatic hyphenation points using loaded patterns and exceptions for language 0.
- [x] `\showthe` - Displays the value produced by implemented `\the` targets through World's terminal/log effect sink.
- [ ] `\tracingcommands` - Logs command execution when positive.
- [ ] `\tracinglostchars` - Logs missing font characters when positive.
- [ ] `\tracingmacros` - Logs macro expansion and arguments when positive.
- [ ] `\tracingonline` - Mirrors diagnostic log output to the terminal when positive.
- [ ] `\tracingoutput` - Logs shipped-out box contents when positive.
- [ ] `\tracingpages` - Logs page builder calculations when positive.
- [ ] `\tracingparagraphs` - Logs line-breaking calculations when positive.
- [ ] `\tracingrestores` - Logs save-stack restoration when positive.
- [ ] `\tracingstats` - Logs memory statistics at job end when positive.

## File I/O And Output

- [x] `\closein` - Closes a World input stream slot.
- [x] `\closeout` - Closes a World output stream slot by appending an effect record.
- [x] `\endinput` - Stops reading the current input source after the current line through the lexer input stack; source identity is pinned by World input records and lexer summaries can restore mid-source positions.
- [ ] `\immediate` - Executes the following output operation immediately.
- [x] `\input` - Pushes a driver-provided source at the current input position. `tex-expand` scans the file name and calls a source hook; it does not read the filesystem directly.
- [ ] `\inputlineno` - Current line number in the active input file.
- [x] `\openin` - Opens a World-backed content-addressed input stream; missing files leave the slot at EOF.
- [x] `\openout` - Opens a World output stream by appending an effect record.
- [ ] `\output` - Token list invoked by the page builder for output routine processing.
- [x] `\read` - Reads one line from an opened World input stream and defines the target control sequence as a no-parameter macro; terminal reads remain unsupported in nonstop batch execution.
- [ ] `\shipout` - Writes a completed box to the DVI output.
- [ ] `\special` - Emits backend-specific material into the DVI stream.
- [ ] `\write` - Writes expanded material to an output stream, normally delayed until shipout.

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
- [ ] `\language` - Selects the current hyphenation language.
- [x] `\lefthyphenmin` - Minimum characters before the first automatic hyphen; consumed by hlist hyphenation and `\showhyphens`.
- [x] `\patterns` - Adds lccode-normalized INITEX-style Liang patterns for language 0 into the snapshot-covered hyphenation trie.
- [x] `\righthyphenmin` - Minimum characters after the last automatic hyphen; consumed by hlist hyphenation and `\showhyphens`.
- [ ] `\setlanguage` - Inserts a language whatsit into the current horizontal list.
- [x] `\uchyph` - Enables hyphenation of words beginning with uppercase letters when positive.

## Insertions And Splits

- [x] `\holdinginserts` - Keeps insertions on the page list during output routine processing when positive.
- [x] `\insert` - Adds internal vertical material to an insertion class, capturing split parameters and participating in page-builder insertion splitting/distribution.
- [x] `\insertpenalties` - Reports split/floating insertion penalties in mainline and held-over insertion count while an output routine is active.
- [x] `\splitbotmark` - Documented stub until the page/split epic stores marks: expands to an empty token list.
- [x] `\splitfirstmark` - Documented stub until the page/split epic stores marks: expands to an empty token list.
- [x] `\splitmaxdepth` - Maximum depth allowed in split boxes; assignable and captured by `\insert`, with the standalone `\vsplit` primitive still pending.
- [x] `\splittopskip` - Top glue inserted in split boxes; assignable and captured by `\insert`, with the standalone `\vsplit` primitive still pending.
- [ ] `\vsplit` - Splits vertical material from a box register to a target height.

## Job Control

- [ ] `\day` - Current day of the month.
- [ ] `\deadcycles` - Number of output routine calls since the last `\shipout`.
- [ ] `\dump` - Writes a format file in INITEX; otherwise ends the job.
- [x] `\end` - Finishes the current batch execution loop for `umber run`; page finalization/output-file behavior is deferred until typesetting and World output land.
- [ ] `\everyjob` - Token list inserted at the start of every job.
- [x] `\jobname` - Expands to the driver-provided job name as rendered character tokens.
- [x] `\mag` - Magnification ratio, scaled by 1000; implemented as an assignable integer parameter used by true-unit scanning.
- [ ] `\maxdeadcycles` - Maximum allowed output routine cycles without shipout.
- [ ] `\month` - Current month number.
- [ ] `\time` - Current minutes after midnight.
- [ ] `\year` - Current year.

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

- [x] `\afterassignment` - Saves a token in snapshot-covered state and inserts it after the next completed assignment; box-assignment firing is deferred until box assignments land.
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

- [x] `\botmark` - Documented stub until the page builder stores marks: expands to an empty token list.
- [x] `\firstmark` - Documented stub until the page builder stores marks: expands to an empty token list.
- [x] `\mark` - Adds frozen mark text to the current list for later page-builder mark handling.
- [x] `\topmark` - Documented stub until the page builder stores marks: expands to an empty token list.

## Math

- [ ] `\above` - Builds a fraction with explicit rule thickness and no delimiters.
- [ ] `\abovedisplayshortskip` - Glue above a display when the preceding line is short.
- [ ] `\abovedisplayskip` - Normal glue above a display.
- [ ] `\abovewithdelims` - Builds a generalized fraction with delimiters and explicit rule thickness.
- [ ] `\atop` - Builds a fraction-like stack with no rule and no delimiters.
- [ ] `\atopwithdelims` - Builds a fraction-like stack with delimiters and no rule.
- [ ] `\belowdisplayshortskip` - Glue below a display when short-display spacing is used.
- [ ] `\belowdisplayskip` - Normal glue below a display.
- [ ] `\binoppenalty` - Penalty for line breaks after binary operators in math.
- [x] `\defaultskewchar` - Default `\skewchar` value for newly loaded fonts; implemented as an integer parameter used when initializing font banks.
- [x] `\delcode` - Reads or assigns a character's delimiter code; assignments use the code-table facade and bump generations.
- [ ] `\delimiter` - Adds a delimiter by numeric delimiter code.
- [ ] `\delimiterfactor` - Scaling factor used to choose delimiter sizes.
- [ ] `\delimitershortfall` - Allowed shortfall when choosing delimiter sizes.
- [ ] `\displayindent` - Indentation applied to the current display.
- [ ] `\displaylimits` - Uses default limit placement for large operators.
- [ ] `\displaystyle` - Selects display math style.
- [ ] `\displaywidowpenalty` - Penalty before a display after the penultimate paragraph line.
- [ ] `\displaywidth` - Line width available to the current display.
- [ ] `\eqno` - Adds a right-side equation number to a display.
- [ ] `\everydisplay` - Token list inserted when entering display math.
- [ ] `\everymath` - Token list inserted when entering inline math.
- [ ] `\fam` - Current math family for variable-family math characters.
- [ ] `\left` - Starts a delimited math subformula with scalable left delimiter.
- [ ] `\leqno` - Adds a left-side equation number to a display.
- [ ] `\limits` - Forces limits above and below a large operator.
- [ ] `\mathaccent` - Adds a math accent atom.
- [ ] `\mathbin` - Treats the following item as a binary operator atom.
- [ ] `\mathchar` - Adds a math character by numeric math code.
- [x] `\mathchardef` - Defines a control sequence as a math character command usable as an internal integer.
- [ ] `\mathchoice` - Provides alternatives for display, text, script, and scriptscript styles.
- [ ] `\mathclose` - Treats the following item as a closing atom.
- [x] `\mathcode` - Reads or assigns a character's math code; assignments use the code-table facade and bump generations.
- [ ] `\mathinner` - Treats the following subformula as an inner atom.
- [ ] `\mathop` - Treats the following item as a large-operator atom.
- [ ] `\mathopen` - Treats the following item as an opening atom.
- [ ] `\mathord` - Treats the following item as an ordinary atom.
- [ ] `\mathpunct` - Treats the following item as a punctuation atom.
- [ ] `\mathrel` - Treats the following item as a relation atom.
- [ ] `\mathsurround` - Extra space inserted around inline math.
- [ ] `\medmuskip` - Medium math glue between math atoms.
- [ ] `\mkern` - Adds a math kern.
- [ ] `\mskip` - Adds math glue.
- [x] `\muskip` - Reads or assigns a math glue register, including sparse e-TeX indices.
- [x] `\muskipdef` - Defines a symbolic name for a math glue register.
- [ ] `\nolimits` - Forces limits to the side of a large operator.
- [ ] `\nonscript` - Suppresses following glue or kern in script styles.
- [ ] `\nulldelimiterspace` - Width reserved for missing delimiters.
- [ ] `\over` - Builds a normal fraction with no delimiters.
- [ ] `\overline` - Places a rule over the following math item.
- [ ] `\overwithdelims` - Builds a normal fraction with delimiters.
- [ ] `\postdisplaypenalty` - Penalty inserted after a display.
- [ ] `\predisplaypenalty` - Penalty inserted before a display.
- [ ] `\predisplaysize` - Effective width of the line preceding a display.
- [ ] `\radical` - Builds a radical atom from a delimiter code and nucleus.
- [ ] `\relpenalty` - Penalty for line breaks after relation atoms in math.
- [ ] `\right` - Ends a delimited math subformula with scalable right delimiter.
- [ ] `\scriptfont` - Font used for a family in script style.
- [ ] `\scriptscriptfont` - Font used for a family in scriptscript style.
- [ ] `\scriptscriptstyle` - Selects scriptscript math style.
- [ ] `\scriptspace` - Extra space after subscripts and superscripts.
- [ ] `\scriptstyle` - Selects script math style.
- [x] `\skewchar` - Font-specific Env-backed character used to position math accents.
- [ ] `\textfont` - Font used for a family in text style.
- [ ] `\textstyle` - Selects text math style.
- [ ] `\thickmuskip` - Thick math glue between math atoms.
- [ ] `\thinmuskip` - Thin math glue between math atoms.
- [ ] `\underline` - Places a rule under the following math item.
- [ ] `\vcenter` - Builds a vertically centered box for math formulas.

## Page Builder

- [ ] `\hoffset` - Horizontal offset added to the default one-inch origin.
- [ ] `\maxdepth` - Maximum depth allowed on the main vertical page.
- [ ] `\pagedepth` - Depth of the last box on the current page.
- [ ] `\pagefilllstretch` - Third-order infinite stretch currently on the page.
- [ ] `\pagefillstretch` - Second-order infinite stretch currently on the page.
- [ ] `\pagefilstretch` - First-order infinite stretch currently on the page.
- [ ] `\pagegoal` - Target height for the current page.
- [ ] `\pageshrink` - Finite shrink currently on the page.
- [ ] `\pagestretch` - Finite stretch currently on the page.
- [ ] `\pagetotal` - Natural height accumulated on the current page.
- [ ] `\topskip` - Glue inserted before the first box on a page.
- [ ] `\voffset` - Vertical offset added to the default one-inch origin.
- [ ] `\vsize` - Target page body height.

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
- [ ] `\exhyphenpenalty` - Penalty for line breaks after explicit hyphens.
- [x] `\floatingpenalty` - Penalty for insertions floating after their class has split on the current page.
- [ ] `\hyphenpenalty` - Penalty for line breaks at discretionary hyphens.
- [x] `\interlinepenalty` - Penalty inserted between paragraph lines by post-line-break surgery.
- [x] `\lastpenalty` - Reports the last penalty on the current list, or zero.
- [x] `\linepenalty` - Base demerit contribution for each broken line; consumed by the paragraph line breaker.
- [ ] `\outputpenalty` - Penalty value that triggered the current output routine.
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

- [ ] `\cr` - Ends an alignment row.
- [ ] `\crcr` - Ends an alignment row unless a row just ended.
- [ ] `\everycr` - Token list inserted after `\cr` or nonredundant `\crcr`.
- [ ] `\halign` - Builds a horizontal alignment.
- [ ] `\noalign` - Inserts vertical material between alignment rows.
- [ ] `\omit` - Ignores the current alignment entry template.
- [ ] `\span` - Combines adjacent alignment columns.
- [ ] `\tabskip` - Glue inserted between alignment columns.
- [ ] `\valign` - Builds a vertical alignment.
