#ifndef INSTALLATIONCONTROLLER_H
#define INSTALLATIONCONTROLLER_H

#include <QJsonArray>
#include <QJsonObject>
#include <QObject>
#include <QPointer>
#include <QString>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <QVariantList>
#include <QVariantMap>
#include <QtQml/qqmlregistration.h>
#include <functional>

#include "Registrar.h"

class DeviceModel;
class DeviceRegistry;
class DeviceSession;
class LinkController;
class QJSEngine;
class QQmlEngine;
struct FfiCore;

// -- WP INSTQ -- Setting up the installation the hub keeps (MPLAN 1.4): the
// units the hub sees that are not in it, joining them, each unit's room, role
// and feed, the senders and fan-outs, rooms, zones and groups. The logic is
// the core's (dspi_installation_*, dspi-core alink/installation.rs); this only
// keeps the draft for QML and runs the calls.
//
// The draft is the core's JSON {revision, base, model}. The view and the save
// ask the hub over the network, so they run on a thread of this controller's
// own (as SystemController's do); an edit, the validation and the changes are
// worked out from the draft alone and are made at once.
//
// The view is read while the panel is shown, every few seconds: it brings the
// units seen meanwhile, and, with nothing edited, the installation as it is
// now. Once the hub's revision moves past an edited draft's, or a save is
// refused as a conflict, `stale` withdraws Save and QML offers Reload, which
// makes a new draft of the fresh view: the old draft is never given the new
// revision (there is no 3-way merge, so that would undo the other's save).
//
// What the hub's people and devices are called is text from elsewhere: every
// name shown goes through clean().
class InstallationController : public QObject {
    Q_OBJECT
    QML_ELEMENT
    QML_SINGLETON
    // A panel is on show (panelShown()): the view is read while one is
    Q_PROPERTY(bool shown READ shown NOTIFY shownChanged)
    // A hub session answered with the installation
    Q_PROPERTY(bool available READ available NOTIFY viewChanged)
    Q_PROPERTY(QString role READ role NOTIFY viewChanged)
    // Admin, and not read only on this console; `editable` also: nothing on
    // its way, not stale, the draft is of a view that was read
    Q_PROPERTY(bool mayEdit READ mayEdit NOTIFY viewChanged)
    Q_PROPERTY(bool editable READ editable NOTIFY editableChanged)
    // Why not ("" when it may)
    Q_PROPERTY(QString editNote READ editNote NOTIFY viewChanged)
    // The hub keeps an inventory; false: `unseen` is empty without meaning none
    Q_PROPERTY(bool inventory READ inventory NOTIFY viewChanged)
    // The units the hub sees that the draft does not hold: { serial, name,
    // shownSerial, online, outputs }; and their serials, notifying only when
    // one came or went (the rows: what is typed in one stays)
    Q_PROPERTY(QVariantList unseen READ unseen NOTIFY unseenChanged)
    Q_PROPERTY(QStringList unseenSerials READ unseenSerials NOTIFY unseenSerialsChanged)
    // The draft's elements by id, each list notifying only when it changed, so
    // an edit does not rebuild the rows (or take the focus from one); a row
    // reads its element() again when `generation` moves
    Q_PROPERTY(QStringList unitIds READ unitIds NOTIFY unitIdsChanged)
    Q_PROPERTY(QStringList feederIds READ feederIds NOTIFY feederIdsChanged)   // senders, then fan-outs
    Q_PROPERTY(QStringList roomIds READ roomIds NOTIFY roomIdsChanged)
    Q_PROPERTY(QStringList zoneIds READ zoneIds NOTIFY zoneIdsChanged)
    Q_PROPERTY(QStringList groupIds READ groupIds NOTIFY groupIdsChanged)
    Q_PROPERTY(int generation READ generation NOTIFY draftChanged)
    Q_PROPERTY(bool dirty READ dirty NOTIFY draftChanged)
    Q_PROPERTY(bool stale READ stale NOTIFY draftChanged)
    // What saving would change, in the core's sentences
    Q_PROPERTY(QStringList changes READ changes NOTIFY draftChanged)
    // The hub's validator found something (one thing per check): issueOf says it on
    // its field; `issue` is it in the hub's words, "" for none
    Q_PROPERTY(QString issue READ issue NOTIFY draftChanged)
    // Why the view could not be read ("" when it was)
    Q_PROPERTY(QString error READ error NOTIFY viewChanged)
    // -- WP LAY5b -- The edits a Reload of a stale draft left out, in the
    // core's sentences; empty for none. remake() makes them again on the
    // installation as it is now, where the core judges each again.
    Q_PROPERTY(QStringList lost READ lost NOTIFY draftChanged)
    // A save is on its way
    Q_PROPERTY(bool busy READ busy NOTIFY editableChanged)
    // The last save, in words; and whether it went wrong
    Q_PROPERTY(QString message READ message NOTIFY messageChanged)
    Q_PROPERTY(bool messageFailed READ messageFailed NOTIFY messageChanged)
    // -- WP FEATUI -- FEAT1's audio link sources, programme sources and scenes
    // by id, as the lists above
    Q_PROPERTY(QStringList sourceIds READ sourceIds NOTIFY sourceIdsChanged)
    Q_PROPERTY(QStringList programmeIds READ programmeIds NOTIFY programmeIdsChanged)
    Q_PROPERTY(QStringList sceneIds READ sceneIds NOTIFY sceneIdsChanged)
    // The units whose own slots the draft's zone sources replace: { id, name,
    // zone, source } (names); empty for none. The notice before Save
    Q_PROPERTY(QVariantList rerouted READ rerouted NOTIFY draftChanged)
    // After a save that re-routed units: how many play the new channels after
    // an alignment (0: none, or aligned since)
    Q_PROPERTY(int alignPending READ alignPending NOTIFY messageChanged)
public:
    InstallationController(const AppContext &ctx, QObject *parent = nullptr);
    ~InstallationController() override;
    static InstallationController *create(QQmlEngine *, QJSEngine *);

    bool shown() const { return m_panels > 0; }
    // -- WP LAY5b -- A panel shows, or no longer does: a page and a window may
    // each have one, so this counts them
    Q_INVOKABLE void panelShown(bool shown);
    bool available() const { return m_available; }
    QString role() const { return m_role; }
    bool mayEdit() const { return writable() && m_available && m_mayEdit; }
    bool editable() const { return mayEdit() && m_current && !m_stale && m_busy == 0; }
    QString editNote() const;
    bool inventory() const { return m_inventory; }
    QVariantList unseen() const { return m_unseen; }
    QStringList unseenSerials() const { return m_unseenSerials; }
    QStringList unitIds() const { return m_unitIds; }
    QStringList feederIds() const { return m_feederIds; }
    QStringList roomIds() const { return m_roomIds; }
    QStringList zoneIds() const { return m_zoneIds; }
    QStringList groupIds() const { return m_groupIds; }
    int generation() const { return m_generation; }
    bool dirty() const { return m_dirty; }
    bool stale() const { return m_stale; }
    QStringList changes() const { return m_changes; }
    QString issue() const { return m_issue; }
    QStringList lost() const { return m_lostChanges; }   // -- WP LAY5b --
    QString error() const { return m_error; }
    bool busy() const { return m_busy > 0; }
    QString message() const { return m_message; }
    bool messageFailed() const { return m_messageFailed; }
    int revision() const { return m_draft.value(QLatin1String("revision")).toInt(); }
    // -- WP FEATUI --
    QStringList sourceIds() const { return m_sourceIds; }
    QStringList programmeIds() const { return m_programmeIds; }
    QStringList sceneIds() const { return m_sceneIds; }
    QVariantList rerouted() const { return m_rerouted; }
    int alignPending() const { return m_alignPending; }

    // The reads below take `generation` for QML's sake alone: passed, a binding
    // runs again when it moves (an unused read of it, "(generation, element(id))",
    // is dropped by the QML compiler). They ignore it.
    //
    // An element of the draft: { id, kind, name (clean, its id without one),
    // and as its kind has them: serial, hubSerial (-- WP LAY5b -- as the hub
    // gives it, to compare and never to show), room, role (receiver, repeater,
    // spdif_follower, spdif_source, usb_direct), upstream, spdifSource, txMask,
    // rooms, units, source }. Empty for none
    Q_INVOKABLE QVariantMap element(const QString &id, int generation = 0) const;
    // The entry of `unseen` with this serial, empty for none
    Q_INVOKABLE QVariantMap unseenUnit(const QString &serial, int generation = 0) const;
    // What an element's field can be, { id, name } each. `value` is kept in
    // the list when it is no longer one (a removed feed)
    Q_INVOKABLE QVariantList roomChoices(int generation = 0) const;
    Q_INVOKABLE QVariantList sourceChoices(int generation = 0) const;
    Q_INVOKABLE QVariantList feedChoices(const QString &role, const QString &selfId, const QString &value, int generation = 0) const;
    // The issue on a field of an element ("upstream", "room", "role", "name";
    // "": its other fields), "" for none. An edit the core refused is said on
    // the field it was made on
    Q_INVOKABLE QString issueOf(const QString &id, const QString &field, int generation = 0) const;

    // The edits: false when refused (issueOf says why on its field; a joining
    // unit's id is "serial:" and its serial, a new element's "new:" and its kind)
    Q_INVOKABLE bool join(const QString &serial, const QString &name, const QString &as, const QString &room, const QString &upstream);
    Q_INVOKABLE bool rename(const QString &id, const QString &name);
    Q_INVOKABLE bool setRoom(const QString &id, const QString &room);
    Q_INVOKABLE bool setRole(const QString &id, const QString &role);
    Q_INVOKABLE bool setUpstream(const QString &id, const QString &upstream);
    Q_INVOKABLE bool setSpdifSlot(const QString &id, int slot, bool on);
    // A zone's room or a group's unit, in or out
    Q_INVOKABLE bool setMember(const QString &id, const QString &member, bool in);
    Q_INVOKABLE bool addRoom(const QString &name);
    Q_INVOKABLE bool addZone(const QString &name, const QString &source);
    Q_INVOKABLE bool addGroup(const QString &name);
    Q_INVOKABLE bool addSender(const QString &name);
    Q_INVOKABLE bool remove(const QString &id);

    // -- WP FEATUI -- FEAT1's edits (dspi-core alink/installation.rs). element()
    // also gives a unit's `slots` (8 numbers, 255 silent; empty: its own),
    // `slotsZone`/`slotsSource` (the zone and source that own them, names; ""
    // when hand-made), a source's `sender` and `channels` ("8 9", from 0 as the
    // core's sentences say them), a programme source's `programmeKind`,
    // `feeds`, `inputs` ("0 1") and a
    // scene's `actions` ([{ text, op }]) and `systemMute`.
    // A zone's source: its audio link units play the source's channels
    Q_INVOKABLE bool setZoneSource(const QString &zone, const QString &source);
    // The names of the audio link units a zone's source would route
    Q_INVOKABLE QStringList zoneUnits(const QString &zone, int generation = 0) const;
    // A unit's slots: 8 entries, a channel 0..63 or 255 for silent; empty: its own again
    Q_INVOKABLE bool setSlots(const QString &unit, const QVariantList &inputs);
    Q_INVOKABLE QVariantList senderChoices(int generation = 0) const;
    // Channels as typed: "8 9", "8-11", "0, 2"
    Q_INVOKABLE bool addSource(const QString &name, const QString &sender, const QString &channels);
    Q_INVOKABLE bool setSource(const QString &id, const QString &sender, const QString &channels);
    // kind: computer_usb, spdif, analogue, alink_sender; inputs as channels above
    Q_INVOKABLE QVariantList programmeKinds() const;
    Q_INVOKABLE QVariantList programmeFeedChoices(const QString &kind, const QString &value, int generation = 0) const;
    Q_INVOKABLE bool addProgramme(const QString &name, const QString &kind, const QString &feeds, const QString &inputs);
    Q_INVOKABLE bool setProgramme(const QString &id, const QString &kind, const QString &feeds, const QString &inputs);
    // A scene with no actions yet, and its actions: op mute, unmute,
    // master_volume (value in dB) or source (target a zone, `source` its source)
    Q_INVOKABLE bool addScene(const QString &name);
    Q_INVOKABLE QVariantList sceneTargetChoices(int generation = 0) const;
    Q_INVOKABLE bool addSceneAction(const QString &scene, const QString &op, const QString &target, double value, const QString &source);
    Q_INVOKABLE bool removeSceneAction(const QString &scene, int index);
    // The alignment that puts saved slots live was asked (SystemController)
    Q_INVOKABLE void alignAsked();

    Q_INVOKABLE void save();
    // Drop the edits for the installation as last read, and read it again
    Q_INVOKABLE void reload();
    // -- WP LAY5b -- The edits of `lost`, all or none (false, and `message`
    // says why); or leave them
    Q_INVOKABLE bool remake();
    Q_INVOKABLE void dropLost();
    Q_INVOKABLE void refresh();

    // `text` as it is safe to show: no control or format characters (bidi
    // overrides, zero-width ones), so a name cannot make a sentence read other
    // than it is
    static QString clean(const QString &text);
    // -- For the smoke driver: the device held read only on this console
    void setReadOnlyForTest(bool on);

signals:
    void shownChanged();
    void viewChanged();
    void editableChanged();
    void unseenChanged();
    void unseenSerialsChanged();
    void unitIdsChanged();
    void feederIdsChanged();
    void roomIdsChanged();
    void zoneIdsChanged();
    void groupIdsChanged();
    void draftChanged();
    void messageChanged();
    void sourceIdsChanged();      // -- WP FEATUI --
    void programmeIdsChanged();
    void sceneIdsChanged();

private:
    bool writable() const;
    LinkController *link();
    QString hubKey();
    void updatePolling();
    void applyView(const QJsonObject &o, const QString &hub);
    void takeDraft(const QJsonObject &draft);
    // The lists, the issue and the changes of m_draft; signals
    void draftMoved();
    void updateUnseen();
    // `edits` on the draft, for `field` of `id`
    bool edit(const QJsonArray &edits, const QString &id, const QString &field);
    bool refuse(const QString &id, const QString &field, const QString &why);
    void setMessage(const QString &message, bool failed);
    QJsonObject model() const { return m_draft.value(QLatin1String("model")).toObject(); }
    QJsonObject find(const QString &id, QString *kind = nullptr) const;
    QString label(const QString &id) const;

    FfiCore *m_core;
    DeviceModel *m_device;
    // -- WP MDQT2b -- as SystemController: the active device's core, taken
    // again on a switch, when the hub's identity (LinkController's, the
    // active one's too) and the core change together; `m_epoch` drops answers
    // asked before it
    DeviceRegistry *m_registry;
    QPointer<DeviceSession> m_session;
    quint64 m_epoch = 0;
    void rebind();
    QPointer<LinkController> m_link;
    QThreadPool m_pool;
    QTimer m_timer;
    bool m_shutDown = false;
    bool m_readOnlyForTest = false;
    bool m_wasWritable = true;
    int m_panels = 0;   // -- WP LAY5b --
    bool m_reading = false;
    int m_busy = 0;

    // The view
    bool m_available = false;
    bool m_mayEdit = false;
    bool m_inventory = false;
    QString m_role;
    QString m_error;
    QString m_hub;            // the hub it was read from
    QJsonArray m_seen;        // its unseen, as read
    QJsonObject m_latest;     // its draft

    // The draft
    QJsonObject m_draft;
    bool m_current = false;   // of a view read since the last save
    bool m_dirty = false;
    bool m_stale = false;
    int m_generation = 0;
    QStringList m_changes;
    // -- WP LAY5b -- the edits made on the draft, and those a Reload left out
    QJsonArray m_edits, m_lost;
    QStringList m_lostChanges;
    QString m_issue, m_issueSubject, m_issueField, m_issueShown;
    QString m_refusedId, m_refusedField, m_refusedWhy;
    QVariantList m_unseen;
    QStringList m_unseenSerials;
    QStringList m_unitIds, m_feederIds, m_roomIds, m_zoneIds, m_groupIds;

    QString m_message;
    bool m_messageFailed = false;
    // -- WP FEATUI --
    bool editScene(const QString &scene, const std::function<void(QJsonArray &)> &change);
    QStringList m_sourceIds, m_programmeIds, m_sceneIds;
    QVariantList m_rerouted;
    int m_alignPending = 0;
};

#endif // INSTALLATIONCONTROLLER_H
