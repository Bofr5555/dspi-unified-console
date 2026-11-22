# -- WP INSTQ -- Setting up the hub's installation (joining units, rooms, roles,
# feeds, zones): the Installation tab of Window > System, which is a panel now
list(APPEND DSPI_APP_SOURCES
    src/controllers/InstallationController.h src/controllers/InstallationController.cpp)
list(APPEND DSPI_QML_FILES
    qml/system/SystemPanel.qml
    qml/system/InstallationPanel.qml qml/system/InstallationUnitCard.qml
    qml/system/InstallationJoinCard.qml qml/system/InstallationMembersCard.qml
    qml/system/InstallationField.qml qml/system/InstallationChoice.qml
    qml/system/InstallationName.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeInstallation.cpp)
