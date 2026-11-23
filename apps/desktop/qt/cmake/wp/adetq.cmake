# -- WP ADETQ -- Speaker auto-detect on the Qt desktop (AUTODES wave 3a): the
# main screen's line for a kept layout that is not confirmed, and the smoke
# sections adet-speakers and adet-agent.
list(APPEND DSPI_QML_FILES qml/onboarding/SpeakersLayoutNote.qml)
list(APPEND DSPI_SMOKE_SOURCES smoke/SmokeAdetq.cpp)
