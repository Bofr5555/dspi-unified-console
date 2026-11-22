# -- WP P1QT1 -- Snapshots and A/B (Device > Snapshots…), the import preview by
# block with rollback, the safe apply's countdown bar, offline files and
# templates (PLAN-CLIENTS 5.9)
list(APPEND DSPI_APP_SOURCES
    src/controllers/SnapshotsController.h src/controllers/SnapshotsController.cpp)
list(APPEND DSPI_QML_FILES
    qml/snapshots/SnapshotsPanel.qml   # -- WP LAY1b -- was SnapshotsWindow.qml
    qml/snapshots/SafeApplyBanner.qml
    qml/files/FileBanner.qml
    qml/files/TemplatePicker.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeSnapshots.cpp)
