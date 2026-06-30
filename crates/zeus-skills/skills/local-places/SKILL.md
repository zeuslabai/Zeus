# local-places

Search for local businesses, restaurants, and places using map APIs.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a local places discovery assistant. Help users find restaurants, businesses, attractions, and services nearby. Provide details like hours, ratings, reviews, and directions.

## Tools

### places_search
Search for places near a location.
```json
{
  "type": "object",
  "properties": {
    "query": {
      "type": "string",
      "description": "Search query (e.g., 'coffee shop', 'Italian restaurant')"
    },
    "location": {
      "type": "string",
      "description": "Location (address, city, or 'lat,lng')"
    },
    "radius": {
      "type": "integer",
      "default": 5000,
      "description": "Search radius in meters"
    },
    "type": {
      "type": "string",
      "description": "Place type filter (restaurant, cafe, gas_station, etc.)"
    }
  },
  "required": ["query", "location"]
}
```

### places_details
Get detailed information about a place.
```json
{
  "type": "object",
  "properties": {
    "place_id": {
      "type": "string",
      "description": "Google Places ID"
    }
  },
  "required": ["place_id"]
}
```

### places_nearby
Find places of a type nearby.
```json
{
  "type": "object",
  "properties": {
    "location": {
      "type": "string"
    },
    "type": {
      "type": "string",
      "description": "Place type (restaurant, hospital, pharmacy, etc.)"
    },
    "radius": {
      "type": "integer",
      "default": 1500
    },
    "open_now": {
      "type": "boolean",
      "default": false
    }
  },
  "required": ["location", "type"]
}
```

### places_reviews
Get reviews for a place.
```json
{
  "type": "object",
  "properties": {
    "place_id": {
      "type": "string"
    }
  },
  "required": ["place_id"]
}
```

### places_hours
Get opening hours for a place.
```json
{
  "type": "object",
  "properties": {
    "place_id": {
      "type": "string"
    }
  },
  "required": ["place_id"]
}
```

### places_directions
Get directions to a place.
```json
{
  "type": "object",
  "properties": {
    "origin": {
      "type": "string"
    },
    "destination": {
      "type": "string"
    },
    "mode": {
      "type": "string",
      "enum": ["driving", "walking", "bicycling", "transit"],
      "default": "driving"
    }
  },
  "required": ["origin", "destination"]
}
```

## Commands

### search
```bash
curl -s "https://maps.googleapis.com/maps/api/place/textsearch/json?query={query}&location={location}&radius={radius}&key=$GOOGLE_PLACES_API_KEY"
```

### details
```bash
curl -s "https://maps.googleapis.com/maps/api/place/details/json?place_id={place_id}&fields=name,formatted_address,formatted_phone_number,opening_hours,rating,reviews,website&key=$GOOGLE_PLACES_API_KEY"
```

### nearby
```bash
curl -s "https://maps.googleapis.com/maps/api/place/nearbysearch/json?location={location}&radius={radius}&type={type}&key=$GOOGLE_PLACES_API_KEY"
```

### directions
```bash
curl -s "https://maps.googleapis.com/maps/api/directions/json?origin={origin}&destination={destination}&mode={mode}&key=$GOOGLE_PLACES_API_KEY"
```

## Environment
- GOOGLE_PLACES_API_KEY

## Permissions
- network
