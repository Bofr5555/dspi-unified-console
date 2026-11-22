#ifndef SESSIONSTATE_H
#define SESSIONSTATE_H

#include <QString>
#include <QVariant>
#include <memory>

#include "DeviceSession.h"

// -- WP MDQT2a -- A controller's own state of one device, typed, in the
// session's stash(): made on first use, gone with the session (and with the
// last job that holds it).
//
//     struct State { ... };
//     std::shared_ptr<State> st = sessionState<State>(m_ctx.activeSession(), QStringLiteral("XController"));
//
// Ask for it at each use: the active session is another one after a switch. A
// job asked of one device keeps the State it was asked with, so what it brings
// back is that device's, whichever is active by then.
template <class T>
std::shared_ptr<T> sessionState(DeviceSession *session, const QString &controller) {
    if (!session) return std::make_shared<T>();
    QVariant &held = session->stash(controller)[QStringLiteral("state")];
    auto state = held.value<std::shared_ptr<T>>();
    if (!state) {
        state = std::make_shared<T>();
        held = QVariant::fromValue(state);
    }
    return state;
}

#endif // SESSIONSTATE_H
