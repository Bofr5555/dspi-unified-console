# Object names: ADETQ (speaker auto-detect, Qt)

Smoke sections: `adet-speakers` (order 986) and `adet-agent` (order 987), `smoke/SmokeAdetq.cpp`.

- `SpeakersLayoutNote`, `SpeakersLayoutNote.open`: the main screen's line for a kept layout that is not confirmed
  (a Registrar::Blocking banner: Classic's banners, Modern's status strip).
- `SpeakersPage.stamp`: when the kept layout was confirmed, detected or kept.
- `SpeakersPage.detect`: Detect…; the page's `detectDialog` property is its question (`SpeakersPage.detectQuestion`).
- `SpeakersPage.detecting`, `SpeakersPage.detectText`, `SpeakersPage.stopDetect`: while the app's detection runs.
- `SpeakersPage.detection`, `SpeakersPage.summary`, `SpeakersPage.details`, `SpeakersPage.detailsTable`,
  `SpeakersPage.detail.<row>.<column>` (columns: output, found, heard at, top falls, −6 dB at, lowest played).
- `Speakers.pill.<output>`: a row's marker in SpeakersTable (the step and the page).
- `Inspector.nothing`: the channel inspector's note for an output the detection heard nothing from.
- `AiConfirmSheet.warning`: the agent question's warning on the detection's mechanism.
- `AiAgents.detect` (`.switch`): Settings › AI Agents › Agents may detect speakers; the page's `detectDialog` property
  is its question (`AiAgents.detectQuestion`).
