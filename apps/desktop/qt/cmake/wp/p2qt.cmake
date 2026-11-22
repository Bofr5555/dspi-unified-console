# -- WP P2QT -- PRO phase 2 in Qt: the hub's shared snapshots (SnapshotsController's
# hub section), Device > Events… (this console's log and the hub's history),
# the hub's alarms as notices (NoticeCenter), Device > Identify; HubFeed hands
# the hub's events to all of them
list(APPEND DSPI_APP_SOURCES
    src/controllers/HubFeed.h src/controllers/HubFeed.cpp
    src/controllers/EventsController.h src/controllers/EventsController.cpp
    src/controllers/IdentifyController.h src/controllers/IdentifyController.cpp)
list(APPEND DSPI_QML_FILES qml/events/EventsPanel.qml)   # -- WP LAY1b -- was EventsWindow.qml
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeHubPages.cpp)
