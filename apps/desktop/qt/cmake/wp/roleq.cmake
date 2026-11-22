# -- WP ROLEQ -- The speaker layout: the Getting Started step (its words are
# assets/onboarding/speakers.json, a resource of its own at
# :/onboarding/speakers.json, so Android's getting_started.json stays as it is),
# Settings › Speakers, and the banner the Measure tabs show.
list(APPEND DSPI_APP_SOURCES
    src/controllers/SpeakersController.h src/controllers/SpeakersController.cpp)
list(APPEND DSPI_QML_FILES
    qml/onboarding/SpeakersTable.qml
    qml/onboarding/SpeakersStep.qml
    qml/onboarding/SpeakersBanner.qml
    qml/settings/SpeakersPage.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeRoleq.cpp)
get_filename_component(DSPI_ROLEQ_ASSETS "${CMAKE_CURRENT_SOURCE_DIR}/../../../assets/onboarding" ABSOLUTE)
function(dspi_roleq_target target)
    qt_add_resources(${target} dspi_roleq_speakers
        PREFIX "/onboarding"
        BASE "${DSPI_ROLEQ_ASSETS}"
        FILES "${DSPI_ROLEQ_ASSETS}/speakers.json")
endfunction()
list(APPEND DSPI_APP_HOOKS dspi_roleq_target)
