#ifndef NAMESCONTROLLER_H
#define NAMESCONTROLLER_H

#include <QHash>
#include <QObject>
#include <QString>
#include <QVariantList>
#include <QtQml/qqmlregistration.h>
#include <memory>

#include "HardwareController.h"
#include "Registrar.h"

// -- WP QP11 -- The names the device keeps for itself: its own name (Global
// Parameters > Device Name) and one per input source (Inputs > Source Names).
// The Mac's SettingsSaveCoordinator names category: typing edits a draft, an
// entry equal to the stored name is dropped, and the save bar writes every
// changed name (sources in order, then the device's). Names are what the
// device stores and the front panel draws: printable ASCII, at most 31
// characters. The drafts are each device's own; firmware without names hides
// the rows (the core's feature probe answers dspi_cached_*_name).
class NamesController : public QObject, public SaveParticipant {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(bool sourceNamesSupported READ sourceNamesSupported NOTIFY changed)
    Q_PROPERTY(bool deviceNameSupported READ deviceNameSupported NOTIFY changed)
    // [{source, title, detail, stored, text, edited}] for the sources the
    // device offers now, in the source menu's order
    Q_PROPERTY(QVariantList sources READ sources NOTIFY changed)
    Q_PROPERTY(QString deviceName READ deviceName NOTIFY changed)
    Q_PROPERTY(bool deviceNameEdited READ deviceNameEdited NOTIFY changed)
public:
    NamesController(const AppContext &ctx, QObject *parent);
    static NamesController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<NamesController>(); }

    bool sourceNamesSupported() const { return state()->sourcesSupported; }
    bool deviceNameSupported() const { return state()->deviceSupported; }
    QVariantList sources() const;
    QString deviceName() const;
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
    // -- WP MDQT2a -- The names and drafts are the device's own
    struct State {
        QString serial;
        bool sourcesSupported = false;
        bool deviceSupported = false;
        QHash<int, QString> stored;      // by source
        QHash<int, QString> drafts;      // typed, not saved
        QString storedDevice;
        QString deviceDraft;
        bool deviceDraftSet = false;
    };
    std::shared_ptr<State> state() const;
    void onSnapshot();
    // Reads `session`'s names into its own State
    void readNames(DeviceSession *session, const std::shared_ptr<State> &st);
    QList<int> sourceOptions() const;
    QHash<int, QString> pendingSources() const;
    const QString *pendingDeviceName() const;
    void touched();

    AppContext m_ctx;
    std::shared_ptr<State> m_shown;   // -- WP MDQT2a -- the State last announced
};

#endif // NAMESCONTROLLER_H
