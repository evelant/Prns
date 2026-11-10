import CPrnsHost

func nativeInterfaceConfig(
    _ value: InterfaceConfig,
    arena: NativeArena
) throws -> PrnsInterfaceConfig {
    var result = PrnsInterfaceConfig()
    result.struct_size = MemoryLayout<PrnsInterfaceConfig>.size
    switch value {
    case .autoLan(
        let groupId,
        let discoveryScope,
        let discoveryPort,
        let dataPort,
        let devices,
        let ignoredDevices,
        let multicastAddressType
    ):
        result.kind = InterfaceConfigKind.autoLan.rawValue
        if let groupId {
            result.has_group_id = 1
            result.group_id = try arena.string(groupId)
        }
        if let discoveryScope {
            result.has_discovery_scope = 1
            result.discovery_scope = discoveryScope.rawValue
        }
        if let discoveryPort {
            result.has_discovery_port = 1
            result.discovery_port = discoveryPort
        }
        if let dataPort {
            result.has_data_port = 1
            result.data_port = dataPort
        }
        let nativeDevices = try devices.map { try arena.string($0) }
        result.devices = try arena.array(nativeDevices).map { UnsafePointer($0) }
        result.device_count = nativeDevices.count
        let nativeIgnoredDevices = try ignoredDevices.map { try arena.string($0) }
        result.ignored_devices = try arena.array(nativeIgnoredDevices).map { UnsafePointer($0) }
        result.ignored_device_count = nativeIgnoredDevices.count
        if let multicastAddressType {
            result.has_multicast_address_type = 1
            result.multicast_address_type = multicastAddressType.rawValue
        }
    case .tcpClient(let target, let bitrate):
        result.kind = InterfaceConfigKind.tcpClient.rawValue
        result.target = try arena.string(target)
        try apply(bitrate, to: &result)
    case .tcpServer(let bind, let bitrate):
        result.kind = InterfaceConfigKind.tcpServer.rawValue
        result.bind = try arena.string(bind)
        try apply(bitrate, to: &result)
    case .udp(let local, let peer, let bitrate):
        result.kind = InterfaceConfigKind.udp.rawValue
        result.local = try arena.string(local)
        result.peer = try arena.string(peer)
        try apply(bitrate, to: &result)
    case .serial(let port, let line):
        result.kind = InterfaceConfigKind.serial.rawValue
        result.port = try arena.string(port)
        result.line = nativeSerialLine(line)
    case .kiss(
        let port,
        let line,
        let flowControl,
        let preambleMillis,
        let transmitTailMillis,
        let persistence,
        let slotTimeMillis,
        let stationCallsign,
        let stationIntervalSeconds
    ):
        result.kind = InterfaceConfigKind.kiss.rawValue
        result.port = try arena.string(port)
        result.line = nativeSerialLine(line)
        result.flow_control = flowControl ? 1 : 0
        result.preamble_millis = preambleMillis
        result.transmit_tail_millis = transmitTailMillis
        result.persistence = persistence
        result.slot_time_millis = slotTimeMillis
        try apply(stationCallsign, stationIntervalSeconds, to: &result, arena: arena)
    case .ax25Kiss(
        let port,
        let line,
        let flowControl,
        let preambleMillis,
        let transmitTailMillis,
        let persistence,
        let slotTimeMillis,
        let callsign,
        let ssid
    ):
        result.kind = InterfaceConfigKind.ax25Kiss.rawValue
        result.port = try arena.string(port)
        result.line = nativeSerialLine(line)
        result.flow_control = flowControl ? 1 : 0
        result.preamble_millis = preambleMillis
        result.transmit_tail_millis = transmitTailMillis
        result.persistence = persistence
        result.slot_time_millis = slotTimeMillis
        result.callsign = try arena.string(callsign)
        result.ssid = ssid
    case .rNode(
        let port,
        let radio,
        let flowControl,
        let stationCallsign,
        let stationIntervalSeconds,
        let airtimeLimitShortCentiPercent,
        let airtimeLimitLongCentiPercent
    ):
        result.kind = InterfaceConfigKind.rNode.rawValue
        result.port = try arena.string(port)
        result.radio = nativeRNodeRadio(radio)
        result.flow_control = flowControl ? 1 : 0
        try apply(stationCallsign, stationIntervalSeconds, to: &result, arena: arena)
        if let airtimeLimitShortCentiPercent {
            result.has_airtime_limit_short_centi_percent = 1
            result.airtime_limit_short_centi_percent = airtimeLimitShortCentiPercent
        }
        if let airtimeLimitLongCentiPercent {
            result.has_airtime_limit_long_centi_percent = 1
            result.airtime_limit_long_centi_percent = airtimeLimitLongCentiPercent
        }
    case .multiRNode(let port, let stationCallsign, let stationIntervalSeconds, let members):
        result.kind = InterfaceConfigKind.multiRNode.rawValue
        result.port = try arena.string(port)
        try apply(stationCallsign, stationIntervalSeconds, to: &result, arena: arena)
        let nativeMembers = try members.map { member in
            PrnsMultiRNodeMemberConfig(
                struct_size: MemoryLayout<PrnsMultiRNodeMemberConfig>.size,
                name: try arena.string(member.name),
                virtual_port: member.virtualPort,
                radio: nativeRNodeRadio(member.radio),
                flow_control: member.flowControl ? 1 : 0,
                outgoing: member.outgoing ? 1 : 0
            )
        }
        result.members = try arena.array(nativeMembers).map { UnsafePointer($0) }
        result.member_count = nativeMembers.count
    case .pipe(let command, let respawnDelayMillis):
        result.kind = InterfaceConfigKind.pipe.rawValue
        let nativeCommand = try command.map { try arena.string($0) }
        result.command = try arena.array(nativeCommand).map { UnsafePointer($0) }
        result.command_count = nativeCommand.count
        result.respawn_delay_millis = respawnDelayMillis
    case .backboneClient(let target, let bitrate):
        result.kind = InterfaceConfigKind.backboneClient.rawValue
        result.target = try arena.string(target)
        try apply(bitrate, to: &result)
    case .backboneServer(let bind, let bitrate):
        result.kind = InterfaceConfigKind.backboneServer.rawValue
        result.bind = try arena.string(bind)
        try apply(bitrate, to: &result)
    case .i2p(let peers, let connectable):
        result.kind = InterfaceConfigKind.i2p.rawValue
        let nativePeers = try peers.map { try arena.string($0) }
        result.peers = try arena.array(nativePeers).map { UnsafePointer($0) }
        result.peer_count = nativePeers.count
        result.connectable = connectable ? 1 : 0
    case .weave(let port):
        result.kind = InterfaceConfigKind.weave.rawValue
        result.port = try arena.string(port)
    case .automaticUsb:
        result.kind = InterfaceConfigKind.automaticUsb.rawValue
    case .automaticBluetoothLe:
        result.kind = InterfaceConfigKind.automaticBluetoothLe.rawValue
    case .configuredBluetoothLe(let discoveryGroups):
        result.kind = InterfaceConfigKind.configuredBluetoothLe.rawValue
        let groups = try discoveryGroups.map { try arena.string($0) }
        result.discovery_groups = try arena.array(groups).map { UnsafePointer($0) }
        result.discovery_group_count = groups.count
    case .webSocketClient(let target, let framing):
        result.kind = InterfaceConfigKind.webSocketClient.rawValue
        result.target = try arena.string(target)
        result.websocket_framing_selection = framing.rawValue
    case .webSocketServer(let bind, let framing):
        result.kind = InterfaceConfigKind.webSocketServer.rawValue
        result.bind = try arena.string(bind)
        result.websocket_framing_selection = framing.rawValue
    case .browserRendezvous(let url):
        result.kind = InterfaceConfigKind.browserRendezvous.rawValue
        result.url = try arena.string(url)
    }
    return result
}

func nativeInterfaceRouting(_ value: InterfaceRoutingPolicy) throws -> PrnsInterfaceRoutingPolicy {
    guard value.gravity.map({
        $0 >= HostContract.safeIntMin && $0 <= HostContract.safeIntMax
    }) ?? true else {
        throw StatusFailure(
            operation: "marshalInterfaceRouting",
            status: .invalidArgument
        )
    }
    return PrnsInterfaceRoutingPolicy(
        struct_size: MemoryLayout<PrnsInterfaceRoutingPolicy>.size,
        has_mode: value.mode == nil ? 0 : 1,
        mode: value.mode?.rawValue ?? 0,
        has_gravity: value.gravity == nil ? 0 : 1,
        gravity: value.gravity ?? 0,
        has_recursive_path_requests: value.recursivePathRequests == nil ? 0 : 1,
        recursive_path_requests: value.recursivePathRequests == true ? 1 : 0,
        has_announces_from_internal: value.announcesFromInternal == nil ? 0 : 1,
        announces_from_internal: value.announcesFromInternal == true ? 1 : 0,
        has_announces_to_internal: value.announcesToInternal == nil ? 0 : 1,
        announces_to_internal: value.announcesToInternal == true ? 1 : 0
    )
}

private func apply(
    _ value: Bitrate,
    to result: inout PrnsInterfaceConfig
) throws {
    let bitrate = try value.native
    result.bitrate_kind = bitrate.kind
    result.bitrate_bps = bitrate.bitsPerSecond
}

private func apply(
    _ callsign: String?,
    _ interval: UInt64?,
    to result: inout PrnsInterfaceConfig,
    arena: NativeArena
) throws {
    if let callsign {
        result.has_station_callsign = 1
        result.station_callsign = try arena.string(callsign)
    }
    if let interval {
        result.has_station_interval_seconds = 1
        result.station_interval_seconds = interval
    }
}

private func nativeSerialLine(_ value: SerialLineConfig) -> PrnsSerialLineConfig {
    PrnsSerialLineConfig(
        struct_size: MemoryLayout<PrnsSerialLineConfig>.size,
        baud: value.baud,
        data_bits: value.dataBits.rawValue,
        parity: value.parity.rawValue,
        stop_bits: value.stopBits.rawValue
    )
}

private func nativeRNodeRadio(_ value: RNodeRadioConfig) -> PrnsRNodeRadioConfig {
    PrnsRNodeRadioConfig(
        struct_size: MemoryLayout<PrnsRNodeRadioConfig>.size,
        frequency_hz: value.frequencyHz,
        bandwidth_hz: value.bandwidthHz,
        tx_power_dbm: value.txPowerDbm,
        spreading_factor: value.spreadingFactor,
        coding_rate: value.codingRate
    )
}
