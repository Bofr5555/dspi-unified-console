# Object names: ROLEQ (the speaker layout)

Smoke section: `roleq-speakers` (order 205, `smoke/SmokeRoleq.cpp`), before the
Measure sections; it ends with the proposal confirmed.

| Where | Names |
|---|---|
| `SpeakersTable` (the step and the page) | `Speakers.row.<n>`, `Speakers.included.<n>`, `Speakers.role.<n>`, `Speakers.speaker.<n>`, `Speakers.group.<n>` (n: the output, 0-based), `Speakers.note` (the proposal's remarks) |
| Getting Started | `GettingStarted.speakers` (the loader), `SpeakersStep`, `SpeakersStep.noDevice`, `.state`, `.confirm`, `.suggestion` |
| Settings › Speakers (page id `speakers`) | `SpeakersPage`, `Speakers.layout`, `SpeakersPage.banner`, `.readOnly`, `.confirm`, `.suggestion`, `.state`, `.said` |
| Measure | `MeasureSetup.speakersBanner`, `MeasureLive.speakersBanner` (each a `SpeakersBanner` with `SpeakersBanner.open`), `MeasureSetup.role.<n>`, `MeasureSetup.speaker.<n>`, `MeasureSetup.layoutSaved`, `MeasureLive.role` |
