# OHD Connect — Pencil Design Spec (Android implementation brief)

This file is the source of truth for the Android Compose rebuild. Pair it with the PNG exports in this directory:

- `KADlx.png` Home v2 · `LURIu.png` Medication v2 · `gZMmO.png` Food v3 · `yBPJe.png` Food search · `FQzfA.png` Symptom log · `tnEmm.png` Measurement log · `H06Ms.png` Recent events · `eKtkU.png` Configure storage · `N00Rs.png` Urine strip · `NsOBH.png` Personal CORD chat · `NMDCn.png` Form builder · `VCokI.png` Settings hub group · `h2nbdC.png` Buttons showcase · `YtZYE.png` Inputs · `YWS0o.png` Cards/list

All Android frames are **390 × 844 dp**. Light theme primary, no theme switching for v1.

---

## 1. Design tokens

| Token | Value | Use |
|---|---|---|
| `ohd-bg` | `#FFFFFF` | Default surface |
| `ohd-bg-elevated` | `#FAFAFA` | Cards, header back, panels |
| `ohd-ink` | `#0A0A0A` | Primary text, primary buttons (surface) |
| `ohd-ink-soft` | `#3A3A3A` | Body emphasis |
| `ohd-line` | `#E5E5E5` | Borders, hairlines |
| `ohd-line-soft` | `#F2F2F2` | Subtle dividers, segmented control bg |
| `ohd-muted` | `#6B6B6B` | Secondary text, icons |
| `ohd-red` | `#E11D2A` | Brand, primary CTA, accents |
| `ohd-red-dark` | `#B5121E` | Destructive |
| `ohd-red-tint` | `#FCE6E8` | Badge bg (e.g. "1 missed") |
| `ohd-success` | `#1F8E4A` | Success/healthy values |
| `ohd-warn` | `#B57500` | Warning |
| `radius-sm` | 4 dp | Small chips, swatches |
| `radius-md` | 8 dp | Buttons, inputs, segments |
| `radius-lg` | 12 dp | Cards, panels |
| `radius-xl` | 16 dp | Larger cards / sheets |
| `size-1`/`2`/`3`/`4`/`5`/`6`/`8`/`12`/`16`/`24` | 4/8/12/16/20/24/32/48/64/96 | Spacing scale |
| `stroke-1`/`-2` | 1 / 2 dp | Hairline / regular borders |
| `font-display` | **Outfit** (200/300/400 weights) | Wordmark, hero numbers, big stats, screen titles |
| `font-body` | **Inter** (400/500/600) | Body, labels, buttons |
| `font-mono` | **JetBrains Mono** (400/500) | Numerics, units, code |

Already provided in app via Google Fonts (matches landing). Add Inter + Outfit + JetBrains Mono to `app/src/main/res/font/` as resource fonts (download .ttf or use `androidx.compose.ui.text.googlefonts.GoogleFont`).

---

## 2. Reusable components (Compose names → Pencil IDs)

Build these once in `ui/components/` (or a separate design-system module). Each component name on the right is the Pencil node id for cross-referencing.

### `OhdButton` — Primary / Ghost / Secondary / Destructive (`Bk8Xc`/`Vqjiu`/`Y1cID`/`t2Rjme`)
- Height **40 dp**, padding `[h=20, v=0]`, corner `radius-md`, label `Inter 14 / 500`.
- **Primary**: fill `ohd-red`, label `#FFFFFF`. Used for main CTAs.
- **Ghost**: transparent, 1.5 dp `ohd-red` border, label `ohd-red`. Used as inline alt actions.
- **Secondary**: transparent, 1.5 dp `ohd-line` border, label `ohd-ink`. Used for neutral actions.
- **Destructive**: fill `ohd-red-dark`, label `#FFFFFF`. Used for "Revoke", "Delete".
- All buttons can be `fill_container` width or content-sized.

### `OhdInput` (`SipDH`)
- Height **44 dp**, padding `[h=12, v=0]`, corner `radius-md`, 1.5 dp `ohd-line` border, fill `ohd-bg`.
- Placeholder text `Inter 14 / normal / ohd-muted`.

### `OhdField` (`d19IvB`) — labelled input
- Vertical column, `gap=6`, label-input-helper.
- Label: `Inter 13 / 500 / ohd-ink`.
- Input: as `OhdInput`, `fill_container` width.
- Helper: `Inter 12 / normal / ohd-muted`.

### `OhdCard` (`eOWkh`)
- Vertical, padding 16, corner `radius-lg`, fill `ohd-bg-elevated`, 1 dp `ohd-line-soft` border, gap 8.
- Title: `Inter 15 / 600 / ohd-ink`.
- Body: `Inter 13 / normal / ohd-muted`, `fill_container` width with wrap.

### `OhdListItem` (`z99kMg`)
- Horizontal, padding `[v=14, h=16]`, gap 12, alignItems center, fill `ohd-bg`.
- Left text block (vertical, gap 2): primary `Inter 14 / 500 / ohd-ink`, secondary `Inter 12 / normal / ohd-muted`.
- Right meta text: `Inter 14 / normal / ohd-muted`. Often "→" or "+" or "›" or ageing strings ("Today 09:14").
- Used heavily on Recent events, Food results, Measurements list.

### `OhdSectionHeader` (`O0Y1Aj`)
- Padding `[v=8, h=16]`, fill `ohd-bg`.
- Label: `Inter 11 / 500 / ohd-muted`, `letterSpacing=2`, **uppercase content** (e.g. "QUICK LOG", "RECENT", "PRESCRIBED").

### `OhdDivider` (`jcCm7`)
- 1 dp `ohd-line` rule, `fill_container` width, with `padding[h=16]` on the wrapper.

### `OhdTopBar` (`kaowR`)
- Height **52 dp**, padding `[h=16]`, fill `ohd-bg`, bottom border 1 dp `ohd-line`.
- Layout: 20 dp Lucide back icon + flexible centered title `Inter 17 / 500` + right-side action text `Inter 15 / 500 / ohd-red`.
- Back icon hides when isRoot. Action hides when none.

### `OhdTabItem` (`CbMHS`)
- Vertical, gap 3, justifyContent center, height 62, width 80.
- Icon **22 dp** Lucide (`house`/`plus`/`history`/`settings`), label `Inter 10 / normal / letterSpacing 0.5`, **uppercase content**.
- Inactive: icon + label `ohd-muted`. Active: icon + label `ohd-red`.

### `OhdBottomTabBar` (`QALVh`)
- Height 62, fill `ohd-bg`, top border 1 dp `ohd-line`. Four tabs `fill_container` each: HOME / LOG / SETTINGS / HISTORY.
  > **Note:** Pencil `tab2` icon uses `plus` and label "LOG" — i.e. log-entry shortcut. `tab3` is HISTORY (icon `history`). `tab4` is SETTINGS.

### `OhdStatTile` (`A47LgC`)
- Vertical, padding 16, corner `radius-lg`, fill `ohd-bg-elevated`, gap 4.
- Value: `Outfit 32 / 200 / ohd-ink` (e.g. "847", "12,847"). Label: `Inter 12 / normal / ohd-muted`.
- Width 160 by default; pair two side-by-side with `fill_container`.

### `OhdQuickLogItem` (`cA0S5`)
- Horizontal, padding `[h=16]`, gap 12, height 80, alignItems center.
- Corner `radius-lg`, fill `ohd-bg`, 1 dp `ohd-line` border.
- Left icon **22 dp** Lucide (`pill`/`utensils`/`activity`/`thermometer` etc.), tinted `ohd-red`. Label `Inter 15 / 500 / ohd-ink`.

### `OhdMedLogItem` (`hAKak`)
- Horizontal, padding `[v=14, h=16]`, gap 12, alignItems center, width 358.
- Left text block: name (`Inter 14 / 500 / ohd-ink`) + sub (`Inter 12 / normal / ohd-muted`).
- Right "Log" button: 60 × 32 frame, corner `radius-md`, fill `ohd-red`, label "Log" `Inter 12 / 500 / #FFFFFF`. When already taken, the button uses fill `ohd-bg` + 1 dp `ohd-line` border + label "Taken" `ohd-muted`.

### `OhdNutriGauge` (`xEama`)
- Vertical, gap 6, width 80, height 96, alignItems center.
- Top: 76 × 76 circle (donut) — outer track `ohd-line-soft`, sweep arc colored per status (`ohd-muted` = ok, `ohd-ink` = light, `ohd-red` = exceeded). Inside: value (e.g. "73g") + percent (e.g. "66%") on top of a small "/110g".
- Bottom label: `Inter 11 / normal / ohd-muted` (Carbs / Protein / Fat / Sugar).
- Sweep angle is `360 × (value / target)` clamped negatively (CCW).
- Used in Food v3 nutrition panel — a row of 4 gauges.

### `OhdToggle` (`sDhPx`/`j7xy3C`)
- 44 × 24, corner radius 12. Off: fill `ohd-line` with white knob 18 × 18 at x=3. On: fill `ohd-red` with knob at x=23.

### Wordmark / brand mark
- Hero "OHD" in Outfit 200 weight, letter-spacing 8 px, fill `ohd-red`.
- Compact mark ("O–H–D" with bars) is the geometric SVG already in landing. Reuse from existing app SVG resource if convenient.

---

## 3. Navigation model

**Bottom tab bar (4 tabs):** HOME · LOG · SETTINGS · HISTORY

| Tab | Root screen | Icon |
|---|---|---|
| HOME | `KADlx` Home v2 | `lucide:house` |
| LOG | quick-pick sheet OR jumps directly to last-used logger | `lucide:plus` |
| HISTORY | `H06Ms` Recent events | `lucide:history` |
| SETTINGS | `qHoLS` Settings hub | `lucide:settings` |

Top-bar pattern: each non-root screen gets a top bar with back arrow (`lucide:arrow-left`), centered title, and an optional right-side action ("Done"/"Save"/"Log"/"Library"/"Export"…).

Operator-flavor screens that ship today (Setup, Grants, Pending, Cases, Audit, Emergency, Export) **migrate into Settings → Profile & Access**:
- Settings/Hub row "Profile & Access" → list with: Grants (existing GrantsScreen), Pending approvals (PendingScreen), Cases (CasesScreen), Audit (AuditScreen), Emergency (EmergencySettingsScreen), Export (ExportScreen).
- The original Setup is reframed as "Storage & Data" → existing SetupScreen content reflowed against `eKtkU` (Configure Storage) layout.

---

## 4. Per-screen specs

> Every screen is `Box(Modifier.fillMaxSize().background(ohd-bg))` containing top-bar + `Column(verticalScroll)` body + bottom-tab-bar (only on tab roots; pushed-stack screens use only top bar).

### 4.1 Home v2 — `KADlx`

**Header row** (`l3AI7`, padding `[t=16, b=8, h=20]`, alignItems center, gap 12):
- "OHD" `Outfit 28 / 200 / ohd-red`.
- Spacer `fill_container`.
- Lucide icon `sparkles` 22 dp `ohd-muted` (taps → CORD chat).
- Lucide icon `bell` 22 dp `ohd-muted`.

**Body** (`juksJ`, padding `[t=4, b=16, h=16]`, vertical gap 20):

1. **Time-range selector** (`x8nPv`) — segmented control. 4 segments `fill_container` each, height 32, corner `radius-md`, container fill `ohd-line-soft`.
   - Active segment: fill `ohd-ink`, label `Inter 12 / 500 / #FFFFFF`.
   - Inactive: transparent, label `Inter 12 / normal / ohd-muted`.
   - Labels: Today / Week / Month / Year. Default: Today.

2. **Stat row** (`r2zoK`) — 2× `OhdStatTile` `fill_container`, gap 10. Default copy: "847" / "events today", "3" / "devices syncing".

3. **Section header**: "QUICK LOG" (use `OhdSectionHeader`).

4. **Quick-log grid** — two rows (`w2V2D`, `rA3PP`) of 2 `OhdQuickLogItem` each, gap 12.
   - `Medication` `pill` (default), `Food` `utensils`, `Measurement` `activity`, `Symptom` `thermometer`.
   - Tapping each navigates to the matching logger.

5. **Favourites header** (`T3h55`) — horizontal: label "FAVOURITES" `Inter 11 / 600 / ohd-muted letterSpacing 1.5` + spacer + "+ Add" `Inter 12 / ohd-red` link.

6. **Favourites row** (`S97si`, gap 8) — chips: 28 dp height, corner radius 20 dp, fill `ohd-bg-elevated`, 1 dp `ohd-line` border, padding `[v=8, h=12]`, gap 6 between icon (16 dp lucide `ohd-red`) and label (`Inter 13 / normal / ohd-ink`). Default: "Glucose" `droplets`, "Blood pressure" `heart-pulse`.

### 4.2 Medication v2 — `LURIu`

Top bar: title "Medications", right action "Library".

Body (`NANKy`, vertical, no top padding — uses internal sub-headers):

1. **Prescribed header** (`MLDXM`, padding `[v=12, h=16]`): "PRESCRIBED" section label + spacer + red badge "1 missed" — fill `ohd-red-tint`, padding `[v=3, h=8]`, corner radius 20 dp, label `Inter 11 / 500 / ohd-red`.
2. `OhdMedLogItem` x2 (`presc1`, `presc2`):
   - "Metformin 500 mg" / "Prescribed · twice daily · due now" — Log button **active** (red).
   - "Lisinopril 10 mg" / "Prescribed · once daily · taken 8h ago" — Log button **Taken** (white border, muted label).
   - Divider between.
3. **On-hand header** (`O7ycQ`, padding `[t=14, b=8, h=16]`): "ON-HAND".
4. `OhdMedLogItem` x2:
   - "Vitamin D3 2000 IU" / "Last taken · 3 days ago" — Log button white-border style.
   - "Omega-3 1000 mg" / "Last taken · today" — same.
   - Divider between.
5. **Spacer** `fill_container` height (pushes footer down).
6. **Footer** (`hVye0`, padding `[v=12, h=16]`, top 1 dp border): full-width Ghost button "+ Add to on-hand".

### 4.3 Recent events — `H06Ms`

Top bar: title "Recent Events", no right action.

Body (`wtyqd`):
1. Section header "LAST 50 ENTRIES" (padding `[v=10, h=16]`).
2. List of `OhdListItem`s, divider between each. Sample copy:
   - "Medication · Metformin 500 mg" — "Today 09:14"
   - "Measurement · Glucose 5.4 mmol/L" — "Today 08:47"
   - "Food · Oat porridge with banana" — "Today 08:05"
   - "Symptom · Fatigue 2/5" — "Yesterday 22:30"
   - "Medication · Lisinopril 10 mg" — "Yesterday 21:00"
   - "Measurement · Blood pressure 118/76" — "Yesterday 08:30"
   - … (load real data; cap to 50).

Top bar uses bottom border; body scrolls. Each list-item meta text is the timestamp, primary text is "<EventType> · <human summary>".

### 4.4 Configure storage — `eKtkU` (becomes Setup / Storage&Data settings)

Header (`B0K22`, padding `[v=16, h=20]`): bare title — replaces the top bar for the onboarding flow. (Inside settings stack, use the regular OhdTopBar with title "Storage & Data".)

Body (`USjkT`, padding `[v=8, h=20]`, gap 20):
1. Heading: "Where should OHD store your data?" `Outfit 22 / 300 / ohd-ink, lineHeight 1.3, fill_container width`.
2. Subtitle: "You can change this at any time. Your data is always your property regardless of where it lives." `Inter 13 / normal / ohd-muted, lineHeight 1.5`.
3. Four option cards. Each card (corner `radius-lg`, fill `ohd-bg`, 1 dp `ohd-line` border, padding 16, horizontal layout, gap 12):
   - 20 dp ellipse radio (selected: filled `ohd-ink`; unselected: empty with 1.5 dp `ohd-line` border).
   - 22 dp Lucide icon (color matches `ohd-ink` when selected, `ohd-muted` when unselected).
   - Vertical text block: title `Inter 15 / 500`, desc `Inter 12 / normal / ohd-muted`.
   - **Selected card** also has 1.5 dp `ohd-ink` border AND an `opt1Expanded` panel inside (vertical, padding `[t=0, b=16, h=16]`, fill `ohd-bg-elevated`, gap 12):
     - Explainer text `Inter 12 / normal / ohd-muted, lineHeight 1.5`: "Data is saved as a single file on your device. The file grows as you log more entries — typically a few MB per year. You can set a retention limit below."
     - Management row (justifyContent end, gap 10, alignItems center): label "Keep data for" `Inter 13 / normal / ohd-ink` + chip "Forever ▾" (1 dp `ohd-line` border, padding `[v=6, h=12]`, corner `radius-sm`, label `Inter 13 / normal / ohd-ink`).
   - The four options are:
     1. **On this device** — `lucide:smartphone`, "Stored locally. No account, no network." (default selected).
     2. **OHD Cloud** — `lucide:cloud`, "Synced across devices. Requires network."
     3. **Self-hosted** — `lucide:server`, "Your own server. Full control."
     4. **Provider hosted** — `lucide:building-2`, "Via your insurer, employer or clinic."
4. **Notice card** — corner `radius-md`, fill `ohd-bg-elevated`, padding 12, gap 8: 16 dp `lucide:shield-check` `ohd-muted` + text `Inter 12 / normal / ohd-muted, lineHeight 1.5`: "Switching storage later migrates all your data. Nothing is lost. Your data is always exportable as an encrypted OHD archive — easily converted to JSONL. Full format spec in the docs (link coming)."
5. Primary button **Continue** `fill_container`.

### 4.5 Settings hub — `qHoLS` (within `VCokI` group)

Top bar: title "Settings", no back, no action.

Body: sequential rows (no padding between, each row has its own padding `[v=14, h=16]`, height ~52 dp, fill `ohd-bg`, bottom 1 dp `ohd-line-soft` separator). Each row: 20 dp Lucide icon `ohd-ink` + label `Inter 15 / 500 / ohd-ink` (`fill_container`) + 20 dp `lucide:chevron-right` `ohd-muted`.

Rows in order:
1. `database` — **Storage & Data** → Storage sub-screen.
2. `shield` — **Profile & Access** → Access sub-screen (which exposes Grants/Pending/Cases/Audit/Emergency/Export operator screens).
3. `file-text` — **Forms & Measurements** → Forms sub-screen (form builder list).
4. `utensils` — **Food & Nutrition** → Food sub-screen.
5. `activity` — **Health Connect** → Health Connect sub-screen.
6. `dumbbell` — **Activities** → Activities sub-screen.
7. `bell` — **Reminders & Calendar** → Reminders sub-screen.
8. `sparkles` — **CORD** → opens Personal CORD chat.

**Sub-screens (BlqLD/i19B3/NpG1B/gILnx/DZvfn/o57vw/HHMX2)** can be implemented as simple "title + a few rows + back arrow" stubs in v1. The Storage sub-screen reuses the `eKtkU` Configure Storage layout. The Access sub-screen lists rows for Grants / Pending approvals / Cases / Audit log / Emergency / Export — each routes to the existing Compose screen.

### 4.6 Food v3 — `gZMmO`

Top bar: title "Food", no right action.

Body (`RNbBL`):
1. **Nutrition panel** (`OcxF7`, fill `ohd-bg-elevated`, padding `[v=14, h=16]`, gap 12, bottom 1 dp `ohd-line` border):
   - Header row: "Today" `Inter 13 / 500 / ohd-ink` + "1,240 / 2,000 kcal" `JetBrains Mono 13 / normal / ohd-muted` (right-aligned).
   - Gauges row (justifyContent space_between, padding `[v=4, h=8]`): 4× `OhdNutriGauge` — Carbs 73g/110g 66%, Protein 48g/80g 60%, Fat 14g/70g 20% (ink), Sugar 28g/20g 140% (red — exceeds target).
2. **Scan area** (`RGkDH`, height 207, layout none, fill `#747474`):
   - Background image (placeholder food/barcode shot — use any sample food image asset until we have a real camera preview).
   - Horizontal red accent line 1 dp tall, opacity 85%.
   - Hint label "Point camera at barcode" `Inter 12 / #FFFFFF / opacity 0.5`.
   - In v1 implementation, we don't ship the camera. Just render a 207 dp area with a placeholder background and the hint text. Tapping it triggers a snackbar "Scanning isn't wired yet — search by name below".
3. **Search row** (`WorTG`, padding `[v=12, h=16]`): full-width `OhdInput` placeholder "Search food or type name…". Tapping it transitions to Food search (`yBPJe`).
4. **Recent section** (`UK7uF`):
   - Section header "RECENT".
   - List of `OhdListItem`s with right-meta "+":
     - "Oat porridge with banana" — "08:05 · 380 kcal"
     - "Greek yoghurt 200 g" — "10:30 · 120 kcal"
     - "Chicken breast 150 g" — "12:45 · 248 kcal"
   - Dividers between.

### 4.7 Food v3 — Search active — `yBPJe`

Top bar: title "Food", no right action.

Body (`LQgvQ`): same nutrition panel as 4.6, then a `fs3SearchRow` (gap 10, padding `[v=12, h=16]`):
- Left: 44 × 44 frame corner `radius-md`, fill `ohd-bg-elevated`, 1 dp `ohd-line` border — contains 20 dp `lucide:scan-barcode` icon `ohd-muted`. Tap → barcode scanner (placeholder).
- Right: `OhdInput` placeholder "Oat porridge…", `fill_container`, focused state (auto-focus).

Section header "RESULTS", then `OhdListItem`s with primary text and source label:
- "Oat porridge with banana" — "380 kcal per serving · OpenFoodFacts"
- "Oat porridge — Quaker" — "352 kcal per serving · OpenFoodFacts"

In v1 we don't ship OpenFoodFacts integration yet; store a small in-app dictionary of ~30 sample foods and filter by name.

### 4.8 Symptom log — `FQzfA`

Top bar: title "Symptom", no right action.

Body (`CbmLP`, padding `[v=20, h=16]`, gap 20):
1. Label "Describe the symptom" `Inter 13 / normal / ohd-muted`.
2. Multiline text-area (height 120, corner `radius-md`, 1.5 dp `ohd-line` border) with placeholder `Inter 14 / ohd-muted, lineHeight 1.5`: "e.g. Mild headache behind the eyes, started after lunch…".
3. Label "Severity" `Inter 13 / ohd-muted`.
4. **5-step chip row** (`MuZxV`, gap 10): 5 chips `fill_container` each, height 44, corner `radius-md`. Selected chip: fill `ohd-ink`, label `Inter 16 / 500 / ohd-bg`. Unselected: 1 dp `ohd-line` border, label `Inter 16 / normal / ohd-muted`. Default selection: 1.
5. Caption row: "Mild" left / "Severe" right, both `Inter 11 / normal / ohd-muted`.
6. Spacer `fill_container`.
7. Primary button "Log symptom" `fill_container`.

### 4.9 Measurement log — `tnEmm`

Top bar: title "Measurement", right action "Log".

Body (`vHYT6`):
1. Section header "QUICK MEASURES".
2. `OhdListItem`s with right-meta "›":
   - "Blood pressure" — "systolic / diastolic · mmHg"
   - "Glucose" — "mmol/L or mg/dL"
   - "Body weight" — "kg"
   - "Body temperature" — "°C or °F"
   - Dividers between.
3. Section header "CUSTOM FORMS" (padding `[t=16, b=8, h=16]`).
4. `OhdListItem`s:
   - "Urine strip" — "8 fields · glucose, protein, pH…" → routes to Urine strip (`N00Rs`).
   - "Pain score" — "2 fields · location, intensity".

Tapping a quick-measure opens a tiny inline "log a value" sheet (out of scope for v1 — for now, route to a placeholder activity with an `OhdField` for the value + Log button).

### 4.10 Urine strip — `N00Rs`

Top bar: title "Urine Strip", right action "Log".

Body (`RoWfW`, vertical scroll):
- **Notice strip** (`WKyQV`, padding `[v=10, h=16]`, fill `ohd-bg-elevated`, gap 8): "Pick the colour closest to your strip. Tap to select." `Inter 12 / normal / ohd-muted`.
- **Field row** (per analyte — repeat 4–8x). Each row (`eZsYl`, vertical, padding 16, gap 10, bottom 1 dp `ohd-line` border):
  - Header (horizontal): name `Inter 14 / 500 / ohd-ink, fill_container` + value `JetBrains Mono 13 / ohd-muted` (e.g. "Negative", "7.0", "—").
  - Swatches row (gap 5–6): N rectangles `fill_container` each, height 36, corner `radius-sm`, fill = the standard analyte colour gradient. **Selected swatch** has a 2 dp `ohd-ink` outside stroke.
  - Optional caption row: low/high labels (Inter 10 / mono / ohd-muted).
  - Default analytes shown: **Glucose** (5 swatches: cream → yellow → light green → green → dark green; labels "Neg" / ">55"); **pH** (6 swatches: orange → light orange → yellow → mint → teal → blue; labels "5" / "9"); **Protein** (4 swatches: cream → khaki → mustard → brown); **Leukocytes** (4 swatches: cream → pink → magenta → purple).
  - Selected indices: Glucose 1 (Negative), pH 4 (7.0 Healthy), Protein 1 (Negative), Leukocytes none (`—`).

### 4.11 Personal CORD chat — `NsOBH`

Top bar (`J2l7im`, height 52, fill `ohd-bg`, bottom border 1 dp `ohd-line`, padding `[h=16]`, gap 8, alignItems center):
- 20 dp `lucide:arrow-left`.
- Title "CORD" `Outfit 17 / 300 / ohd-ink`, centered, `fill_container`.
- Right: chip 12-dp corner radius, fill `ohd-bg-elevated`, padding `[v=4, h=10]`, gap 4: model name "claude-3.5-sonnet" `Inter 11 / ohd-muted` + 12 dp `lucide:chevron-down`. (Tappable model selector.)

Thread (`FvAoZ`, padding 16, vertical gap 16):
- Notice row (centered, gap 6): 14 dp `lucide:database` `ohd-muted` + text "12,847 events · analysing your data" `Inter 11 / ohd-muted`.
- User bubble row (justifyContent end): bubble `cornerRadius [16,16,4,16]`, fill `ohd-ink`, padding 12, max-width ~240 dp, content `Inter 14 / #FFFFFF`.
- Assistant row (gap 8, top-aligned): 28 × 28 circle `ohd-red` (avatar) + content column (`fill_container` to ~88% width):
  - Bubble `cornerRadius [4,16,16,16]`, fill `ohd-bg-elevated`, padding 12, content `Inter 14 / ohd-ink, fill_container`.
  - Optionally a chart card below the bubble: corner radius 8, 1 dp `ohd-line-soft` border, padding 10, gap 6: tiny title `Inter 11 / ohd-muted` + 60 dp tall placeholder rectangle (`ohd-line-soft`) + caption.
- Repeat user/assistant alternation.

Input bar (`ibvA7`, padding `[v=8, h=12]`, top 1 dp border, alignItems center, gap 8):
- `OhdInput`-like bubble: corner radius 20, fill `ohd-bg-elevated`, height 40, padding `[h=12]`, placeholder "Ask anything about your health…".
- Send button: 36 × 36 circle, fill `ohd-red`, content 18 dp `lucide:arrow-up` `#FFFFFF`.

In v1 we don't wire a real LLM. The CORD button on the home header opens this screen, which initially shows the notice + an empty input. Sending a message echoes back a stub answer ("I'm offline in this build — but here's what your data looks like…") with a placeholder chart card. This still demonstrates the UX surface.

### 4.12 Form builder — `NMDCn`

Top bar: title "New Form", right action "Save".

Body (`uR1Dm`, padding `[v=20, h=16]`, gap 20):
1. **Form name row** (vertical, gap 6): label "Form name" `Inter 13 / ohd-muted` + `OhdInput` placeholder "e.g. Urine strip, Pain score…".
2. Section header "FIELDS".
3. **Field row** (one initial entry — corner `radius-md`, padding `[v=14, h=16]`, gap 12, 1 dp `ohd-line` border, alignItems center):
   - 20 dp `lucide:hash` `ohd-muted`.
   - Vertical text block (`fill_container`, gap 2): label "Glucose" `Inter 14 / 500 / ohd-ink`, sub "number · mmol/L" `Inter 12 / normal / ohd-muted`.
   - 20 dp `lucide:grip-vertical` `ohd-muted` (drag handle).
4. Ghost button "+ Add field" `fill_container`.

Saving the form persists it as a custom event type (out of scope for v1 wiring — store JSON in shared prefs and surface in Settings → Forms).

---

## 5. Compose / module shape

Suggested package layout under `connect/android/app/src/main/java/com/ohd/connect/`:

```
ui/
  theme/
    Color.kt        // ohd-bg, ohd-ink, ohd-red, ohd-red-tint, ohd-success, … as `val OhdColors = …`
    Type.kt         // FontFamily.OhdDisplay (Outfit), .OhdBody (Inter), .OhdMono (JetBrainsMono); MaterialTheme typography mapping
    Shapes.kt       // Sm=4, Md=8, Lg=12, Xl=16
    Theme.kt        // OhdTheme { … } wrapping MaterialTheme + LocalContentColor + status bar colors
  components/
    OhdButton.kt    // sealed Variant: Primary/Ghost/Secondary/Destructive
    OhdInput.kt     OhdField.kt
    OhdCard.kt      OhdListItem.kt
    OhdSectionHeader.kt   OhdDivider.kt
    OhdTopBar.kt    OhdBottomTabBar.kt
    OhdStatTile.kt  OhdQuickLogItem.kt
    OhdMedLogItem.kt OhdNutriGauge.kt
    OhdToggle.kt    OhdSegmentedTimeRange.kt
  screens/          // re-fold to Pencil consumer flow
    HomeScreen.kt              // KADlx
    MedicationScreen.kt        // LURIu
    FoodScreen.kt              // gZMmO
    FoodSearchScreen.kt        // yBPJe
    SymptomLogScreen.kt        // FQzfA
    MeasurementScreen.kt       // tnEmm
    UrineStripScreen.kt        // N00Rs
    FormBuilderScreen.kt       // NMDCn
    RecentEventsScreen.kt      // H06Ms (replaces shipped LogScreen list view)
    settings/
      SettingsHubScreen.kt     // qHoLS
      StorageSettingsScreen.kt // Configure Storage layout (eKtkU)
      AccessSettingsScreen.kt  // entry to Grants/Pending/Cases/Audit/Emergency/Export
      … sub-screens (stubs for Forms / Food / HealthConnect / Activities / Reminders)
    cord/
      CordChatScreen.kt        // NsOBH
  // Existing operator-flavor screens stay in place (GrantsScreen, PendingScreen, CasesScreen, AuditScreen, EmergencySettingsScreen, ExportScreen, SetupScreen) — accessed from AccessSettings + StorageSettings.
```

`MainActivity.kt` becomes a `Scaffold` with `OhdTopBar` (when applicable) + `OhdBottomTabBar` + `NavHost`. Use `androidx.navigation:navigation-compose` (already pinned in app/build.gradle.kts per the BUILD.md).

Lucide icons: pull `androidx.compose.material:material-icons-extended` if it has matches, else use the **Phosphor** or **lucide-android** library, OR simplest: include the few SVG paths we need as `ImageVector` resources (about 30 unique icons total). Keep it light.

---

## 6. Migration / what to drop or keep

Keep in v1 (operator screens accessed via Settings → Access):
- `GrantsScreen`, `PendingScreen`, `CasesScreen`, `AuditScreen`, `EmergencySettingsScreen`, `ExportScreen`, `SetupScreen`.

Replace:
- `DashboardScreen` → `HomeScreen` (4.1).
- `LogScreen` (the shipped per-event recent list) → `RecentEventsScreen` (4.3).
- `SettingsScreen` → `SettingsHubScreen` (4.5) + sub-screens.

Add:
- `MedicationScreen`, `FoodScreen`, `FoodSearchScreen`, `SymptomLogScreen`, `MeasurementScreen`, `UrineStripScreen`, `FormBuilderScreen`, `CordChatScreen`.

Wire: bottom tab bar with HOME / LOG / HISTORY / SETTINGS. LOG opens a small bottom-sheet picker (Medication / Food / Measurement / Symptom — same icons as Quick-log) → routes to that logger.

---

## 7. Data wiring (v1 scope)

For the rebuild, **don't change the Rust core or `StorageRepository`**. Map the Pencil screens onto the existing repository surface:

| Pencil screen | Repo call |
|---|---|
| Home stat "events today" | count of events with timestamp today (via `audit_query` filter) |
| Recent events | `audit_query` (limit 50, descending) |
| Quick-log Medication / Food / Symptom / Measurement | `put_event` with the appropriate event type |
| Urine strip Log | composite `put_event` (one event with multiple channels — see existing event vocabulary in `spec/`) |
| Settings → Access → Grants/Pending/etc. | existing repo methods: `list_grants`, `list_pending`, `approve_pending`, `list_cases`, `audit_query`, … |
| Storage settings | reuse SetupScreen logic for picking storage path + keying |

Stub data for Home, Medication, and Food where no real source exists yet (favourites list, on-hand meds, food dictionary). Keep stubs in a single `ui/screens/StubData.kt` so they're easy to delete later.

---

## 8. Acceptance bar

A reviewer with the Pencil PNGs side by side should be able to confirm:
- Top-bar layout / heights / colors match.
- Section headers, list items, dividers visually match (densities, colors, type).
- Quick-log icon set + colors match.
- Stat tiles look like the Pencil exports (Outfit 200 weight numbers).
- Bottom-tab bar shows correct icon set (house, plus, history, settings) and the active tab uses `ohd-red`.
- Configure-storage onboarding screen uses the four-option card layout with selected option expanded.

A working build must:
- Cold-launch to **HomeScreen** (after first-run completes Storage choice).
- Tab navigation between Home / Log-picker / History / Settings works.
- All operator screens reachable via Settings → Access (no regression).
- No crash when opening any of the new screens (stub data is fine).
