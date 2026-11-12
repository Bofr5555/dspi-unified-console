#ifndef NAMESCONTROLLER_H
#define NAMESCONTROLLER_H

#include <QHash>
#include <QObject>
#include <QString>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>

#include "HardwareController.h"
#include "Registrar.h"

// -- WP QP11 -- The names the device keeps for itself: its own name (Global
// Parameters > Device Name) and one per input source (Inputs > Source Names).
// The Mac's SettingsSaveCoordinator names category: typing edits a draft, an
// entry equal to the stored name is dropped, and the save bar writes every
// changed name (sources in order, then the device's). Names are what the
// device stores and the front panel draws: printable ASCII, at most 31
// characters. A device switch drops the drafts; firmware without names hides
// the rows (the core's feature probe answers dspi_cached_*_name).
class NamesController : public QObject, public SaveParticipant {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool sourceNamesSupported READ sourceNamesSupported NOTIFY changed)
    Q_PROPERTY(bool deviceNameSupported READ deviceNameSupported NOTIFY changed)
    // [{source, title, detail, icon, stored, text, edited}] for the sources the
    // device offers now, in the source menu's order
    Q_PROPERTY(QVariantList sources READ sources NOTIFY changed)
    Q_PROPERTY(QString deviceName READ deviceName NOTIFY changed)
    Q_PROPERTY(bool deviceNameEdited READ deviceNameEdited NOTIFY changed)
public:
    NamesController(const AppContext &ctx, QObject *parent);
    static NamesController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<NamesController>(); }

    bool sourceNamesSupported() const { return m_sourcesSupported; }
    bool deviceNameSupported() const { return m_deviceSupported; }
    QVariantList sources() const;
    QString deviceName() const { return m_deviceDraftSet ? m_deviceDraft : m_storedDevice; }
    bool deviceNameEdited() const { return pendingDeviceName() != nullptr; }

    // A person typed: the draft takes the cleaned text (returned, so the field
    // shows what will be stored)
    Q_INVOKABLE QString setSourceName(int source, const QString &text);
    Q_INVOKABLE QString setDeviceName(const QString &text);
    // The built-in name of a source ("USB", "S/PDIF 2"; the Mac's builtInInputSourceTitle)
    Q_INVOKABLE QString builtInTitle(int source) const;

    // SaveParticipant
    bool hasPending() const override;
    void save(std::function<void(const QStringList &)> done) override;
    void revert() override;

signals:
    void changed();

private:
    void onSnapshot();
    void readNames();
    QList<int> sourceOptions() const;
    QHash<int, QString> pendingSources() const;
    const QString *pendingDeviceName() const;
    void touched();

    AppContext m_ctx;
    QString m_serial;
    bool m_sourcesSupported = false;
    bool m_deviceSupported = false;
    QHash<int, QString> m_stored;      // by source
    QHash<int, QString> m_drafts;      // typed, not saved
    QString m_storedDevice;
    QString m_deviceDraft;
    bool m_deviceDraftSet = false;
};

#endif // NAMESCONTROLLER_H
