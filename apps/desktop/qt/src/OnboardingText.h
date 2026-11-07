#ifndef ONBOARDINGTEXT_H
#define ONBOARDINGTEXT_H

#include <QObject>
#include <QVariantList>
#include <QVariantMap>

class DSPiBridge;

// WP QD32: the release notes and the Getting Started wizard's words, from the core,
// which resolves them for the platform it was compiled for. Reads only built-in
// data, so these don't block and need no device.
class OnboardingText : public QObject
{
    Q_OBJECT

public:
    OnboardingText(DSPiBridge *bridge, QObject *parent);

    // [{version, headline, items}], newest first
    Q_INVOKABLE QVariantList releaseNotes() const;
    // Shape: dspi_core::link_text. Empty if the core gave nothing
    Q_INVOKABLE QVariantMap gettingStarted() const;
};

#endif // ONBOARDINGTEXT_H
