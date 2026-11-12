#include "PrefsMap.h"

#include <QSettings>

PrefsMap::PrefsMap(const QString &group, QObject *parent) : QQmlPropertyMap(this, parent), m_group(group) {}

void PrefsMap::declare(const QString &key, const QVariant &fallback) {
    m_fallbacks.insert(key, fallback);
    QSettings settings;
    insert(key, typed(key, settings.value(m_group + QLatin1Char('/') + key, fallback)));
}

QVariant PrefsMap::typed(const QString &key, const QVariant &value) const {
    const QVariant fallback = m_fallbacks.value(key);
    if (!fallback.isValid() || value.metaType() == fallback.metaType()) return value;
    QVariant converted = value;
    // Text that is not a number of the default's type keeps the default
    if (!converted.convert(fallback.metaType())) return fallback;
    if (fallback.metaType().id() == QMetaType::Bool && value.metaType().id() == QMetaType::QString) {
        const QString text = value.toString().trimmed().toLower();
        if (text != QLatin1String("true") && text != QLatin1String("false")) return fallback;
    }
    return converted;
}

QVariant PrefsMap::updateValue(const QString &key, const QVariant &input) {
    const QVariant value = typed(key, input);
    QSettings settings;
    settings.setValue(m_group + QLatin1Char('/') + key, value);
    return value;
}

void PrefsMap::restoreDefaults() {
    QSettings settings;
    for (auto it = m_fallbacks.cbegin(); it != m_fallbacks.cend(); ++it) {
        settings.remove(m_group + QLatin1Char('/') + it.key());
        if (value(it.key()) != it.value()) {
            insert(it.key(), it.value());
            emit valueChanged(it.key(), it.value());
        }
    }
}
