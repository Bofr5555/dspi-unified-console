# Object names: INSTQ (the installation's setup)

| Name | What |
|---|---|
| `SystemWindow.tabs`, `SystemWindow.tab.status`, `SystemWindow.tab.installation` | Window > System's tabs (Advanced, on a hub) |
| `InstallationPanel`, `Installation.body`, `Installation.none` | the panel, what scrolls in it, the line until it is read |
| `Installation.note`, `Installation.unseenNote` | why nothing can be edited; no unit is missing, or the hub keeps no inventory |
| `Installation.unseen.<serial>` | a unit not in the installation ("" the row that takes a typed serial) |
| `Installation.joinName.<serial>`, `.joinAs.`, `.joinRoom.`, `.joinUpstream.`, `Installation.join.<serial>` | what it joins as, and Join; `Installation.joinSerial` the typed serial |
| `Installation.joinIssue.<serial>`, `Installation.joinUpstreamIssue.<serial>` | why it cannot join |
| `Installation.unit.<id>` | a unit's card; `shownName` (name, role) |
| `Installation.name.<id>`, `Installation.remove.<id>` | any element's name and Remove |
| `Installation.room.<id>`, `Installation.role.<id>`, `Installation.upstream.<id>`, `Installation.slot.<id>.<n>` | a unit's room, role, feed, an S/PDIF source's time-base slots |
| `Installation.roomIssue.<id>`, `.roleIssue.`, `.upstreamIssue.`, `Installation.issue.<id>` | the issue on a field, and on the rest of an element |
| `Installation.element.<id>`, `Installation.member.<id>.<member>` | a sender, fan-out, room, zone or group; a zone's room or a group's unit |
| `Installation.new.<kind>`, `Installation.add.<kind>`, `Installation.newIssue.<kind>`, `Installation.newZoneSource` | the rows that add a sender, room, zone or group |
| `Installation.changes` | what saving would change; `shownName` |
| `Installation.issue`, `Installation.stale`, `Installation.message` | what the hub would refuse; changed elsewhere; the last save |
| `Installation.save`, `Installation.reload` | Save; Discard, or Reload when changed elsewhere |
