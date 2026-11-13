# -- WP QP12 -- Control surfaces: Settings > Control Surfaces, Channel Groups,
# Macros and Auxiliary Outputs
list(APPEND DSPI_APP_SOURCES
    src/controllers/CsController.h src/controllers/CsController.cpp)
list(APPEND DSPI_QML_FILES
    qml/settings/cs/CsRows.qml
    qml/settings/cs/CsBadge.qml
    qml/settings/cs/CsPill.qml
    qml/settings/cs/CsApplyRow.qml
    qml/settings/cs/CsCard.qml
    qml/settings/cs/CsBindingCard.qml
    qml/settings/cs/CsRemoteButtons.qml
    qml/settings/cs/CsRemoteButton.qml
    qml/settings/cs/CsDisplay.qml
    qml/settings/cs/CsEmptyState.qml
    qml/settings/cs/CsAddButton.qml
    qml/settings/cs/CsKeys.qml
    qml/settings/cs/CsPageState.qml
    qml/settings/cs/SurfacesPage.qml
    qml/settings/cs/GroupsPage.qml
    qml/settings/cs/MacrosPage.qml
    qml/settings/cs/AuxPage.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeSurfaces.cpp)
