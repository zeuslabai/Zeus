// Zeus Office Layout — zone positions and furniture placement
// Adapted from Star Office UI (MIT License)

const LAYOUT = {
  game: { width: 960, height: 540 },
  zones: {
    breakroom: { x: 160, y: 380, label: "Break Room" },
    desk:      { x: 550, y: 280, label: "Workspace" },
    research:  { x: 750, y: 280, label: "Research" },
    serverroom:{ x: 850, y: 150, label: "Server Room" },
    error:     { x: 300, y: 150, label: "Bug Zone" },
  },
  slots: {
    breakroom: [
      { dx: 0, dy: 0 }, { dx: 50, dy: 10 }, { dx: -40, dy: 15 },
      { dx: 30, dy: -10 }, { dx: -20, dy: 25 }, { dx: 60, dy: -5 },
    ],
    desk: [
      { dx: 0, dy: 0 }, { dx: 70, dy: 0 }, { dx: 140, dy: 0 },
      { dx: 0, dy: 50 }, { dx: 70, dy: 50 }, { dx: 140, dy: 50 },
    ],
    research: [{ dx: 0, dy: 0 }, { dx: 50, dy: 15 }, { dx: -30, dy: 20 }],
    serverroom: [{ dx: 0, dy: 0 }, { dx: 40, dy: 10 }],
    error: [{ dx: 0, dy: 0 }, { dx: 40, dy: 10 }, { dx: -30, dy: 15 }],
  },
  stateZones: {
    idle: "breakroom", writing: "desk", coding: "desk",
    researching: "research", executing: "desk", processing: "desk",
    syncing: "serverroom", deploying: "serverroom", error: "error",
    offline: "breakroom", active: "desk", running: "desk", busy: "desk",
  },
  walkSpeed: 2,
  bubble: { maxWidth: 180, padding: 8, typewriterSpeed: 30, displayTime: 10000 },
};
