# -- WP LAY3 -- The Modern shell: the top bar, the device tabs and switcher, the
# rail, the pages, the dock, the status strip and the notification drawer
list(APPEND DSPI_QML_FILES
    qml/modern/ModernShell.qml qml/modern/TopBar.qml qml/modern/AppMenuButton.qml qml/modern/SplitButton.qml
    qml/modern/DeviceTabs.qml qml/modern/DeviceSwitcher.qml qml/modern/NavRail.qml qml/modern/PageTabs.qml
    qml/modern/PageHost.qml qml/modern/Dock.qml qml/modern/StatusStrip.qml qml/modern/NotificationDrawer.qml
    qml/modern/SectionTable.qml
    qml/modern/pages/OverviewSection.qml qml/modern/pages/EqualiserSection.qml qml/modern/pages/DevicesSection.qml
    qml/main/PresetDialogs.qml)
list(APPEND DSPI_QML_SINGLETONS qml/modern/SectionTable.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeModern.cpp)
