# bird

Identify birds and get birding information using eBird API and Merlin.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a birding assistant. Help users identify birds, find birding hotspots, track recent sightings, and learn about bird species using the eBird API and other ornithological resources.

## Tools

### bird_identify
Identify a bird from description.
```json
{
  "type": "object",
  "properties": {
    "description": {
      "type": "string",
      "description": "Bird description (color, size, location)"
    },
    "location": {
      "type": "string",
      "description": "Region or country code"
    }
  },
  "required": ["description"]
}
```

### bird_recent_sightings
Get recent bird sightings in an area.
```json
{
  "type": "object",
  "properties": {
    "lat": {
      "type": "number"
    },
    "lng": {
      "type": "number"
    },
    "radius": {
      "type": "integer",
      "default": 25,
      "description": "Radius in km"
    },
    "days_back": {
      "type": "integer",
      "default": 7
    }
  },
  "required": ["lat", "lng"]
}
```

### bird_species_info
Get information about a bird species.
```json
{
  "type": "object",
  "properties": {
    "species_code": {
      "type": "string",
      "description": "eBird species code"
    }
  },
  "required": ["species_code"]
}
```

### bird_hotspots
Find birding hotspots nearby.
```json
{
  "type": "object",
  "properties": {
    "lat": {
      "type": "number"
    },
    "lng": {
      "type": "number"
    },
    "radius": {
      "type": "integer",
      "default": 25
    }
  },
  "required": ["lat", "lng"]
}
```

### bird_rare_alerts
Get rare bird alerts for a region.
```json
{
  "type": "object",
  "properties": {
    "region": {
      "type": "string",
      "description": "Region code (e.g., 'US-CA')"
    },
    "days_back": {
      "type": "integer",
      "default": 7
    }
  },
  "required": ["region"]
}
```

### bird_checklist
Get species checklist for a region.
```json
{
  "type": "object",
  "properties": {
    "region": {
      "type": "string"
    }
  },
  "required": ["region"]
}
```

## Commands

### recent_sightings
```bash
curl -s "https://api.ebird.org/v2/data/obs/geo/recent?lat={lat}&lng={lng}&dist={radius}&back={days_back}" \
  -H "X-eBirdApiToken: $EBIRD_API_KEY" | jq '.[] | {comName, sciName, locName, obsDt}'
```

### hotspots
```bash
curl -s "https://api.ebird.org/v2/ref/hotspot/geo?lat={lat}&lng={lng}&dist={radius}" \
  -H "X-eBirdApiToken: $EBIRD_API_KEY" | jq '.[] | {locName, lat, lng, numSpeciesAllTime}'
```

### rare_alerts
```bash
curl -s "https://api.ebird.org/v2/data/obs/{region}/recent/notable?back={days_back}" \
  -H "X-eBirdApiToken: $EBIRD_API_KEY" | jq '.[] | {comName, sciName, locName, obsDt}'
```

### checklist
```bash
curl -s "https://api.ebird.org/v2/product/spplist/{region}" \
  -H "X-eBirdApiToken: $EBIRD_API_KEY"
```

## Environment
- EBIRD_API_KEY

## Permissions
- network
