#ifndef ONBOARDINGCONTROLLER_H
#define ONBOARDINGCONTROLLER_H

#include <QObject>
#include <QPointer>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>

#include "Registrar.h"

class QQmlEngine;
class QJSEngine;

// -- WP QP17 -- Onboarding and the Help menu: the Getting Started wizard and
// What's New from the core's data (dspi_onboarding_*_json, CG10's platform
// wording resolved by the core), the licence block of About (dspi_about_json's
// `licence`), and Help > Save Diagnostics...
//
// - The wizard replaces the console inside the main window (the Mac's rule)
//   while `wizardShown`: on a first start (onboarding/setupDone unset) and from
//   Help > Getting Started... Skipping or finishing sets the flag for good.
// - What's New opens by itself once after an update: the version last shown is
//   onboarding/whatsNewShown; a new install starts caught up. Versions are the
//   firmware's (the notes' own numbering, as on the Mac), capped at the
//   firmware this build ships.
// - The board step installs firmware through `installer`, the firmware
//   package's FirmwareInstaller (QP16), handed over with setInstaller(). What
//   the page reads of it, each optional:
//       state (string): idle, waitingForBoard, waitingForVolume, ready, writing,
//           waitingForDevice, verified, failed, and on the desktop checking,
//           question, inBootloader, unconfirmed
//       chip, volume, version, failure (strings), fraction (0..1), mundane (bool:
//           a failure the user fixes by hand)
//       beginWatching(), stopWatching(), installWhenReady(), installConnected()
//           (restart the open device into its bootloader and install), reset()
//   Without one, the step still tells a connected device's state and hands an
//   update to the Firmware Update window (device.firmwareUpdate).
class OnboardingController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool wizardShown READ wizardShown NOTIFY wizardShownChanged)
    Q_PROPERTY(bool setupDone READ setupDone NOTIFY wizardShownChanged)
    // getting_started.json as the core resolves it
    Q_PROPERTY(QVariantMap gettingStarted READ gettingStarted CONSTANT)
    // [{version, headline, items}], newest first
    Q_PROPERTY(QVariantList releases READ releases CONSTANT)
    Q_PROPERTY(QString whatsNewShown READ whatsNewShown NOTIFY whatsNewShownChanged)
    // The firmware this build ships ("1.1.6"), from its bundled images
    Q_PROPERTY(QString bundledFirmware READ bundledFirmware CONSTANT)
    // about.json's licence: {title, copyright: [..], statement, terms_title,
    // terms_note, terms: [{id, title, text}], freedoms, notices}
    Q_PROPERTY(QVariantMap licence READ licence CONSTANT)
    Q_PROPERTY(bool diagnosticsAvailable READ diagnosticsAvailable CONSTANT)
    Q_PROPERTY(bool savingDiagnostics READ savingDiagnostics NOTIFY savingDiagnosticsChanged)
    Q_PROPERTY(QObject *installer READ installer NOTIFY installerChanged)
public:
    OnboardingController(const AppContext &ctx, QObject *parent);
    static OnboardingController *create(QQmlEngine *, QJSEngine *);

    bool wizardShown() const { return m_wizard; }
    bool setupDone() const;
    QVariantMap gettingStarted() const { return m_gettingStarted; }
    QVariantList releases() const { return m_releases; }
    QString whatsNewShown() const;
    QString bundledFirmware() const;
    QVariantMap licence() const { return m_licence; }
    bool diagnosticsAvailable() const;
    bool savingDiagnostics() const { return m_saving; }
    QObject *installer() const { return m_installer; }
    void setInstaller(QObject *installer);

    // Help > Getting Started...: the wizard, also after setup was done
    Q_INVOKABLE void requestSetup();
    // Skip Setup and the last step's button: the flag is set and stays set
    Q_INVOKABLE void finishSetup();
    // How firmware `deviceVersion` compares with the bundled one: "match",
    // "older" (the device's is older), "newer", or "" when unknown
    Q_INVOKABLE QString firmwareMatch(const QString &deviceVersion) const;
    // The notes not read yet: newer than `shown`, not newer than the firmware
    // this build ships; none for an empty `shown` (a new install)
    Q_INVOKABLE QVariantList unreadSince(const QString &shown) const;
    // What's New was shown: the bundled version counts as read
    Q_INVOKABLE void markWhatsNewRead();
    // After the launch: What's New opens if notes came since the last shown
    // (a new install is marked caught up instead)
    Q_INVOKABLE void showReleaseNotesIfUnread();
    // Advanced > Onboarding (developer): the next start runs setup from the
    // top and starts caught up on the notes again
    Q_INVOKABLE void resetOnboarding();
    // Help > Save Diagnostics...: a save dialog, then the core's bundle
    Q_INVOKABLE void saveDiagnostics();
    // The bundle to `path`, off the GUI thread (the smoke driver calls it)
    Q_INVOKABLE void saveDiagnosticsTo(const QString &path);

signals:
    void wizardShownChanged();
    void whatsNewShownChanged();
    void installerChanged();
    void savingDiagnosticsChanged();
    // A message for the main window (the diagnostics' outcome)
    void notice(const QString &title, const QString &text, bool error);

private:
    AppContext m_ctx;
    bool m_wizard = false;
    bool m_saving = false;
    QVariantMap m_gettingStarted;
    QVariantList m_releases;
    QVariantMap m_licence;
    QPointer<QObject> m_installer;
};

#endif // ONBOARDINGCONTROLLER_H
