# Object names: DL7 (Settings in the Modern design language)

Smoke checks: `dl7Part` in the `lay5b-pages` section (`smoke/SmokeLay5b.cpp`, under `-- WP DL7 --`); pictures
`gallery/dl7-*.png`.

- `<row>.title`, `<row>.field`: a `SettingsSliderRow`'s title and value field in Modern, when the page gives it a
  `title` (`Graphing.lineWidth`, `.gridOpacity`, `.dbRange`, `.dbCenter`, `Spectrum.graphOpacity`, `AiAgents.splCap`).
- `AiAgents.segments` (`.0` … `.2`): Settings › AI Agents › Connect an Agent in Modern (`AiAgents.tabs` keeps the
  choice and shows in Classic).
- `AiAgents.limitsOffer`: the limits offer; Classic's row, Modern's notice (its button `AiAgents.limitsOffer.action`).
- `AiAgents.errorNotice` (`.action`: Try Again), `AiAgents.neverNotice`: Modern's notices for an error and for Never.
- `AiAgents.logEmpty` (`.action`: Show Reads): the page's empty activity in Modern.
- `AiAgentsPanel.empty` (`.action`): the AI Agents activity page's empty state in Modern.
- `WhatsNew.emptyState`: What's New as a Modern page without notes.
