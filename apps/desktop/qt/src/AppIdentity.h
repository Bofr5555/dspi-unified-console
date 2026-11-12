#ifndef APPIDENTITY_H
#define APPIDENTITY_H

#include <QObject>
#include <QString>
#include <QtQml/qqmlregistration.h>

// The names the app shows and files its settings under. The display name is
// set here only; every window title, menu item and dialog builds on it.
namespace AppIdentityNames {
// What people see: window titles, the About page, menu items
inline const char *const displayName = "uConsole for DSPi";
// QSettings' organisation and application, so the settings live in
// $XDG_CONFIG_HOME/uconsole-for-dspi/ and %APPDATA%\uconsole-for-dspi\
// (-- WP RN1 --: new names, no earlier release's settings to carry over)
inline const char *const settingsOrganisation = "uconsole-for-dspi";
inline const char *const settingsApplication = "uconsole-for-dspi";
// The desktop entry's name without .desktop, the binary's name
inline const char *const desktopFileName = "uconsole-for-dspi";
// The project's home page and the firmware's (the Help menu)
inline const char *const projectUrl = "https://github.com/Bofr5555/dspi-unified-console";
inline const char *const firmwareUrl = "https://github.com/WeebLabs/DSPi";
} // namespace AppIdentityNames

// The same names for QML: `AppIdentity.displayName`.
class AppIdentity : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(QString displayName READ displayName CONSTANT)
    Q_PROPERTY(QString version READ version CONSTANT)
    Q_PROPERTY(QString projectUrl READ projectUrl CONSTANT)
    Q_PROPERTY(QString firmwareUrl READ firmwareUrl CONSTANT)
public:
    explicit AppIdentity(QObject *parent = nullptr) : QObject(parent) {}
    static QString displayName() { return QString::fromUtf8(AppIdentityNames::displayName); }
    static QString version() { return QStringLiteral(DSPI_APP_VERSION); }
    static QString projectUrl() { return QString::fromUtf8(AppIdentityNames::projectUrl); }
    static QString firmwareUrl() { return QString::fromUtf8(AppIdentityNames::firmwareUrl); }
};

#endif // APPIDENTITY_H
