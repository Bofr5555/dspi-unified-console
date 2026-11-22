# -- WP LAY0 -- The layout switch (LayoutController, ClassicShell), the panel
# API (Panel, PanelWindow, PanelTable, Shell's page host) with its two pilots
# (Stats, Interrupt Monitor), and Settings > Appearance
list(APPEND DSPI_APP_SOURCES
    src/controllers/LayoutController.h src/controllers/LayoutController.cpp)
list(APPEND DSPI_QML_FILES
    qml/main/ClassicShell.qml qml/main/PanelTable.qml
    qml/components/Panel.qml qml/components/PanelWindow.qml
    qml/tools/StatsPanel.qml qml/tools/MonitorPanel.qml
    qml/settings/pages/AppearancePage.qml)
list(APPEND DSPI_QML_SINGLETONS qml/main/PanelTable.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeLayout.cpp)
