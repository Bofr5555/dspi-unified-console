# -- WP LAY2 -- The popovers' content split from their PopoverPanel hosts, so
# Classic's popover and Modern's inspector show the same content
# (InspectorSection); the smoke section that hosts each content both ways.
list(APPEND DSPI_QML_FILES
    qml/components/InspectorSection.qml
    qml/tools/LimiterSettingsContent.qml
    qml/main/LinkwitzContent.qml
    qml/main/GraphOptionsContent.qml
    qml/main/DashboardLayoutContent.qml
    qml/devices/DevicesContent.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLay2.cpp)
list(APPEND DSPI_SMOKE_QML_FILES smoke/SmokeLay2Window.qml)
