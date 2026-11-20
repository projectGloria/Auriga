// Builds fixtures/snapshots/*.json from the real AX201 record in a collect run.
// Usage: node gen-fixtures.mjs <dev-latest.json> <out dir>
import fs from 'node:fs';
import path from 'node:path';

const [src, out] = process.argv.slice(2);
const raw = JSON.parse(fs.readFileSync(src, 'utf8'));
const snap = raw.snapshot ?? raw;
const ax201 = structuredClone(snap.devices.find((d) => /DEV_43F0/.test(d.instanceId)));
const realtek = structuredClone(
  snap.devices.find((d) => d.class === 'Net' && /VEN_10EC/i.test(d.instanceId)),
);
if (!ax201) throw new Error('no AX201 in source snapshot');

const NOW = '2026-11-20T12:00:00Z';
const WLAN = 'Microsoft-Windows-WLAN-AutoConfig/Operational';
const hoursAgo = (h) => new Date(Date.parse(NOW) - h * 3600e3).toISOString().replace('.000Z', 'Z');

function disconnects(guid, hours, reasons) {
  return hours.map((h, i) => ({
    channel: WLAN,
    provider: 'Microsoft-Windows-WLAN-AutoConfig',
    eventId: 8003,
    level: 4,
    time: hoursAgo(h),
    recordId: 1000 + i,
    data: [
      { name: 'InterfaceGuid', value: guid },
      { name: 'InterfaceDescription', value: 'Wi-Fi adapter' },
      { name: 'ConnectionMode', value: 'Automatic connection with a profile' },
      { name: 'ProfileName', value: 'HomeNet' },
      { name: 'SSID', value: 'HomeNet' },
      { name: 'BSSType', value: 'Infrastructure' },
      { name: 'ReasonCode', value: String(reasons[i % reasons.length]) },
      { name: 'ConnectionId', value: String(10 + i) },
    ],
    message: null,
  }));
}

function base(devices, events) {
  return {
    schemaVersion: 3,
    collectedAt: NOW,
    machine: { osBuild: '26100', osDisplayVersion: '24H2', manufacturer: 'Fixture', model: 'Fixture laptop' },
    devices,
    events: events.sort((a, b) => b.time.localeCompare(a.time)),
    setupapi: [],
    dumps: [],
    eventWindowDays: 30,
    warnings: [],
  };
}

const withPower = (d, allow) => ({ ...d, power: { ...d.power, allowTurnOff: allow, pnpCapabilitiesRaw: allow ? 0 : 24 } });

// 1. AX201 dropping repeatedly; power-off allowed.
const a1 = withPower(ax201, true);
const ev1 = disconnects(a1.netCfgInstanceId, [2, 5, 20, 30, 49, 70, 100, 200], ['7', '7', '2', '8']);
write('ax201-disconnects.json', base([a1, realtek].filter(Boolean), ev1));

// 2. Same adapter, same settings, no disconnects.
write('ax201-healthy.json', base([withPower(ax201, true), realtek].filter(Boolean), []));

// 3. AX210: Intel, same driver family, not an AX201.
const ax210 = withPower(structuredClone(ax201), true);
ax210.instanceId = 'PCI\\VEN_8086&DEV_2725&SUBSYS_00248086&REV_1A\\4&2B1E7F3C&0&00E4';
ax210.name = 'Intel(R) Wi-Fi 6E AX210 160MHz';
ax210.description = ax210.name;
ax210.hardwareIds = [
  'PCI\\VEN_8086&DEV_2725&SUBSYS_00248086&REV_1A',
  'PCI\\VEN_8086&DEV_2725&SUBSYS_00248086',
  'PCI\\VEN_8086&DEV_2725&CC_028000',
  'PCI\\VEN_8086&DEV_2725&CC_0280',
];
ax210.netCfgInstanceId = '{1D3B2F40-77A1-4C6E-9E21-5A0C6F7E8B11}';
const ev3 = disconnects(ax210.netCfgInstanceId, [1, 3, 9, 26, 50, 90, 150], ['7', '4']);
write('ax210-disconnects.json', base([ax210, realtek].filter(Boolean), ev3));

function write(name, obj) {
  fs.writeFileSync(path.join(out, name), JSON.stringify(obj, null, 2) + '\n');
  console.log('wrote', name, obj.devices.length, 'devices', obj.events.length, 'events');
}
