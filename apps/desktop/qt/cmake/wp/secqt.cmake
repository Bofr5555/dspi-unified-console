# -- WP SECQT -- Pairing and key UI: the unsecured line's info button, the new
# key notice, the re-pair question, and their smoke section
list(APPEND DSPI_QML_FILES
    qml/link/UnsecuredInfoButton.qml
    qml/link/KeyNotice.qml
    qml/devices/RePairDialog.qml)
list(APPEND DSPI_SMOKE_SOURCES
    smoke/SmokeSecurity.cpp)
