// -- WP P2QT -- The hub's events, one listener for every controller
#include "HubFeed.h"

#include "Core.h"
#include "CoreApi.h"

#include <QJsonDocument>

HubFeed *HubFeed::of(Core *core) {
    if (!core) return nullptr;
    if (auto *feed = core->findChild<HubFeed *>(QString(), Qt::FindDirectChildrenOnly)) return feed;
    return new HubFeed(core);
}

HubFeed::HubFeed(Core *core) : QObject(core), m_core(core) {
    // Setting it does no I/O; the events come on the Link session's thread
    if (FfiCore *h = core->handle()) dspi_hub_server_set_listener(h, &HubFeed::thunk, this);
}

// Waits for an event in progress, so none reaches this object afterwards (a
// freed core has no handle and sends none)
HubFeed::~HubFeed() {
    if (FfiCore *h = m_core ? m_core->handle() : nullptr) dspi_hub_server_set_listener(h, nullptr, nullptr);
}

// On the core's thread with no core lock held: only post the event home
void HubFeed::thunk(const char *kind, const char *eventJson, void *self) {
    auto *feed = static_cast<HubFeed *>(self);
    const QString k = QString::fromUtf8(kind ? kind : "");
    const QByteArray json(eventJson ? eventJson : "");
    QMetaObject::invokeMethod(
        feed, [feed, k, json] { emit feed->hubEvent(k, QJsonDocument::fromJson(json).object()); }, Qt::QueuedConnection);
}
