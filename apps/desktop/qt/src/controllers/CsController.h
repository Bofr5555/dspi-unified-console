#ifndef CSCONTROLLER_H
#define CSCONTROLLER_H

#include <QHash>
#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QQueue>
#include <QString>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "HardwareController.h"
#include "Poller.h"
#include "Registrar.h"

// -- WP QP12 -- Settings > Control Surfaces, Channel Groups, Macros and
// Auxiliary Outputs (the Mac's ControlSurfacesSettingsTab with its four
// sections): the GPIO controls, the IR receiver with its learned remote
// buttons, the display with its pages, the aux outputs, the groups and the
// macros, all stored on the device.
//
// The core reads the configuration (dspi_cs_fetch, dspi_cs_snapshot), says
// what an editor shows for each record (dspi_cs_describe: rows of pick, number,
// toggle or note) and what an edit makes of it (dspi_cs_edit), and performs
// the operations (dspi_cs_apply). This controller keeps one draft per record:
// an edit changes the draft, Apply sends it, Revert drops it. A draft with no
// edits follows the device. Every call runs on the worker; a failed one shows
// its reason on its card.
//
// What applies when, as on the Mac: a control's, a group's and a macro's edits
// wait for the card's Apply; a new control applies at once; the display's
// settings and pages, an aux output's switch and level apply as they change.
// Everything sent is live on the device and unsaved until the save bar's Save
// (this is its part, order 40, live only); its Revert reloads what is stored.
//
// Records are named by key: "binding/<slot>", "ir/<sub>", "group/<n>",
// "macro/<n>". record(key) gives what a card shows.
class CsController : public QObject, public SaveParticipant {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    Q_PROPERTY(int revision READ revision NOTIFY updated)
    Q_PROPERTY(bool connected READ connected NOTIFY updated)
    Q_PROPERTY(bool writable READ writable NOTIFY updated)
    // The device's tables have been read (false while reading, or without CS)
    Q_PROPERTY(bool loaded READ loaded NOTIFY updated)
    Q_PROPERTY(bool supported READ supported NOTIFY updated)
    Q_PROPERTY(bool busy READ busy NOTIFY updated)
    // The device holds changes not saved to flash
    Q_PROPERTY(bool dirty READ dirty NOTIFY updated)
    Q_PROPERTY(int capsVersion READ capsVersion NOTIFY updated)
    Q_PROPERTY(int slotCount READ slotCount NOTIFY updated)
    Q_PROPERTY(int groupCount READ groupCount NOTIFY updated)
    Q_PROPERTY(int macroCount READ macroCount NOTIFY updated)
    Q_PROPERTY(int maxSteps READ maxSteps NOTIFY updated)
    Q_PROPERTY(int irCount READ irCount NOTIFY updated)
    // Why the last fetch failed, "" when it did not
    Q_PROPERTY(QString loadError READ loadError NOTIFY updated)
    // The remote button a learn is waiting for, -1 for none, and its line
    Q_PROPERTY(int learningSub READ learningSub NOTIFY updated)
    Q_PROPERTY(QString learnMessage READ learnMessage NOTIFY updated)
public:
    CsController(const AppContext &ctx, QObject *parent);
    ~CsController() override;
    static CsController *create(QQmlEngine *, QJSEngine *) { return Registrar::instance<CsController>(); }

    // The firmware's component types (CsType)
    enum Type { None = 0, Button, Switch, Pot, Encoder, Led, LedPwm, Ir, Display, AuxOut, AuxPwm };
    Q_ENUM(Type)

    int revision() const { return m_rev; }
    bool connected() const;
    bool writable() const;
    bool loaded() const { return !m_snap.isEmpty() && supported(); }
    bool supported() const;
    bool busy() const override { return m_ops > 0; }
    bool dirty() const;
    int capsVersion() const;
    int slotCount() const;
    int groupCount() const;
    int macroCount() const;
    int maxSteps() const;
    int irCount() const;
    QString loadError() const { return m_loadError; }
    int learningSub() const { return m_learningSub; }
    QString learnMessage() const { return m_learnMessage; }

    // SaveParticipant: the live preview on the save bar
    bool hasPending() const override { return dirty(); }
    bool liveOnly() const override { return true; }
    void save(std::function<void(const QStringList &failures)> done) override;
    void revert() override;

    // Reads the configuration again (the Control Surfaces page on open)
    Q_INVOKABLE void fetch();
    // The cards a page lists: "controls", "aux", "groups", "macros"
    Q_INVOKABLE QStringList keys(const QString &page) const;
    // What a card shows: {key, kind, slot, type, typeName, name, placeholder,
    // summary, rows, dirty, applying, message, isError, pill, pillColor,
    // inactive, ...}; {} for an unknown key
    Q_INVOKABLE QVariantMap record(const QString &key) const;
    // The component types the add menu of `page` ("controls", "aux") offers:
    // [{type, name}]
    Q_INVOKABLE QVariantList addableTypes(const QString &page) const;
    Q_INVOKABLE QString typeName(int type) const;
    Q_INVOKABLE QString typeIcon(int type) const;
    // A free record of this kind is left
    Q_INVOKABLE bool canAdd(const QString &kind) const;

    // A row of the record's editor changed: an option's value, a number in
    // the row's units, or a bool
    Q_INVOKABLE void edit(const QString &key, const QString &row, const QVariant &value);
    Q_INVOKABLE void setName(const QString &key, const QString &name);
    Q_INVOKABLE void apply(const QString &key);
    Q_INVOKABLE void revertRecord(const QString &key);
    Q_INVOKABLE void remove(const QString &key);
    // Each returns the new record's key, "" when every slot is in use
    Q_INVOKABLE QString addControl(int type);
    Q_INVOKABLE QString addGroup();
    Q_INVOKABLE QString addMacro();
    Q_INVOKABLE void addRemoteButton();

    // Macro steps (`key` a macro)
    Q_INVOKABLE void addStep(const QString &key);
    Q_INVOKABLE void moveStep(const QString &key, int step, int delta);
    Q_INVOKABLE void removeStep(const QString &key, int step);
    Q_INVOKABLE void editStep(const QString &key, int step, const QString &row, const QVariant &value);
    // Runs the stored macro, or stops it while it runs
    Q_INVOKABLE void fire(const QString &key);

    // IR learn for remote button `sub`
    Q_INVOKABLE void startLearn(int sub);
    Q_INVOKABLE void cancelLearn();

    // Aux outputs, at once and never to flash
    Q_INVOKABLE void setAuxState(int slot, bool on);
    Q_INVOKABLE void setAuxLevel(int slot, double percent);

    // The display: its settings' rows, its pages, the panel's state; each
    // change applies at once
    Q_INVOKABLE QVariantMap display() const;
    Q_INVOKABLE void editDisplay(const QString &row, const QVariant &value);
    Q_INVOKABLE void editPage(int page, const QString &row, const QVariant &value);
    Q_INVOKABLE void addPage();
    Q_INVOKABLE void removePage(int page);
    // While a display card shows the panel's state, it is read every second
    Q_INVOKABLE void watchDisplay(bool on);

signals:
    void updated();
    // A card was added; its page opens it
    void added(const QString &key);

private:
    struct Record {
        QString kind;
        int slot = 0;
        QJsonObject live;
        QJsonObject draft;
        QJsonObject description;
        QJsonArray stepDescriptions;   // a macro's, one per step
        QString nameEdit;              // a binding's staged name
        bool nameEdited = false;
        bool applying = false;
        bool editing = false;
        QQueue<std::function<void()>> edits;
        QString message;
        bool isError = false;
    };
    struct Reply {
        bool ran = false;
        bool ok = false;
        QJsonObject json;
        QString failure;   // the device's reason or the core's error
        QByteArray snapshot;
    };

    void onSnapshot();
    void onNotify(uint flags);
    void joinBar();
    void clear();
    void reload();
    void readSnapshot(std::function<void()> then = {});
    void applySnapshot(const QByteArray &text);
    void syncRecords();
    void describeAll();
    void describeOne(const QString &key);
    void bump();
    void beginOp();
    void endOp();
    // Sends `ops` in order through DeviceModel::write (named `what`, its
    // coalescing key), stopping at the first failure, then reads the
    // snapshot; done() gets the outcome on the GUI thread
    void run(const QString &what, const QList<QJsonObject> &ops, std::function<void(const Reply &)> done);
    void queueEdit(const QString &key, std::function<void()> job);
    void nextEdit(const QString &key);
    void finishLearn(const QJsonObject &result, bool timedOut);
    void endLearn();
    void updatePages();
    void notifyPins();

    Record *rec(const QString &key);
    const Record *rec(const QString &key) const;
    QJsonObject liveFor(const QString &kind, int slot) const;
    bool isDirty(const Record &r) const;
    bool irDirty(int sub) const;
    bool anyIrDirty() const;
    bool slotActive(int slot) const;
    bool inUse(const Record &r) const;
    bool isAux(int type) const { return type == AuxOut || type == AuxPwm; }
    int typeCount() const;
    int receiverSlot() const;
    int displaySlot() const;
    QString statusText(int code) const;
    QString nameOf(int slot) const;
    QJsonObject status() const;

    AppContext m_ctx;
    QJsonObject m_snap;
    QHash<QString, Record> m_records;
    QJsonObject m_displayDescription;
    QHash<int, QJsonObject> m_pageDescriptions;
    QHash<int, QString> m_statusTexts;
    QString m_loadError;
    QString m_displayMessage;
    QString m_serial;
    uint64_t m_generation = 0;
    int m_epoch = 0;
    int m_rev = 0;
    int m_ops = 0;
    bool m_fetching = false;
    bool m_joined = false;
    bool m_wasDirty = false;
    int m_learningSub = -1;
    QString m_learnMessage;
    int m_learnTicks = 0;
    int m_macroTicks = 0;
    bool m_watchDisplay = false;
    Poller::Id m_learnPoll = 0;
    Poller::Id m_macroPoll = 0;
    Poller::Id m_displayPoll = 0;
};

#endif // CSCONTROLLER_H
