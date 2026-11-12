#ifndef PREFSMAP_H
#define PREFSMAP_H

#include <QQmlPropertyMap>
#include <QString>
#include <QVariant>
#include <QtQml/qqmlregistration.h>

// App preferences of one settings group ("spectrum"), as properties QML binds
// to and writes: `SettingsController.spectrum.avgMs = 500` stores
// spectrum/avgMs in the app's settings file and every binding follows at once.
// A key declared with a default reads as that type (the INI file keeps
// numbers as text). C++ readers connect valueChanged(key, value).
class PrefsMap : public QQmlPropertyMap {
    Q_OBJECT
    QML_ANONYMOUS
public:
    PrefsMap(const QString &group, QObject *parent = nullptr);

    // Adds `key` with `fallback`, read from the settings file when stored there.
    // Another package adds the keys it owns the same way (graph/height).
    void declare(const QString &key, const QVariant &fallback);
    QVariant fallback(const QString &key) const { return m_fallbacks.value(key); }
    // Every declared key back to its default (and out of the settings file)
    Q_INVOKABLE void restoreDefaults();

protected:
    QVariant updateValue(const QString &key, const QVariant &input) override;

private:
    QVariant typed(const QString &key, const QVariant &value) const;

    QString m_group;
    QVariantHash m_fallbacks;
};

#endif // PREFSMAP_H
