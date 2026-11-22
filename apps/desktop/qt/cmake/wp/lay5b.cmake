# -- WP LAY5b -- Modern's Measure, Library, System, Devices and Settings pages: the
# parts of a panel as page tabs, the Library's presets and files, the device list's
# two halves and the pairing prompt on a page
list(APPEND DSPI_QML_FILES
    qml/devices/DevicesLocal.qml qml/devices/DevicesLink.qml
    qml/devices/PairingContent.qml qml/devices/RePairWords.qml
    qml/modern/pages/DevicesHubs.qml
    qml/modern/pages/LibraryPresets.qml qml/modern/pages/LibraryFiles.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLay5b.cpp)
