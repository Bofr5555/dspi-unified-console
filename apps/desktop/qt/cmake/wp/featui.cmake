# -- WP FEATUI -- The UI for FEAT1-3 and AIVIS: the installation's sources,
# slots, programme sources and scenes; room and zone volume; This computer's
# audio; remembered devices; the agents' SPL cap
list(APPEND DSPI_QML_FILES
    qml/system/InstallationSlotsField.qml qml/system/InstallationSourceCard.qml
    qml/system/InstallationProgrammeCard.qml qml/system/InstallationSceneCard.qml
    qml/system/SystemVolumeCard.qml)
list(APPEND DSPI_APP_SOURCES
    src/controllers/HostAudioController.h src/controllers/HostAudioController.cpp)
list(APPEND DSPI_QML_FILES qml/settings/pages/HostAudioPage.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeFeatui.cpp)
