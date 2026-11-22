#include "Core.h"

#include "DeviceRegistry.h"

Core::Core(DeviceRegistry *registry, QObject *parent) : QObject(parent), m_registry(registry) {
    // The main session hears the whole bus, whichever is active
    connect(registry->mainSession(), &DeviceSession::deviceEvent, this, &Core::deviceEvent);
    connect(registry, &DeviceRegistry::activeChanged, this, &Core::activeChanged);
}

Core::~Core() { shutdown(); }

CoreWorker *Core::worker() { return m_registry->active()->worker(); }

FfiCore *Core::handle() const { return m_registry->active()->handle(); }

CoreWorker *Core::mainWorker() { return m_registry->mainSession()->worker(); }

void Core::startHotplug() { m_registry->startHotplug(); }

void Core::shutdown() { m_registry->shutdown(); }

bool Core::isShutDown() const { return m_registry->isShutDown(); }
