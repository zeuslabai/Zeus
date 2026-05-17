# goplaces

Travel planning with flights, hotels, and itinerary management.

## Version
1.0.0

## Author
Zeus

## System Prompt
You are a travel planning assistant. Help users search for flights, hotels, and create travel itineraries. Provide price comparisons and travel recommendations.

## Tools

### travel_search_flights
Search for flights.
```json
{
  "type": "object",
  "properties": {
    "origin": {
      "type": "string",
      "description": "Origin airport code (e.g., 'LAX')"
    },
    "destination": {
      "type": "string",
      "description": "Destination airport code"
    },
    "departure_date": {
      "type": "string",
      "description": "Departure date (YYYY-MM-DD)"
    },
    "return_date": {
      "type": "string",
      "description": "Return date for round trip"
    },
    "passengers": {
      "type": "integer",
      "default": 1
    },
    "cabin_class": {
      "type": "string",
      "enum": ["economy", "premium_economy", "business", "first"],
      "default": "economy"
    }
  },
  "required": ["origin", "destination", "departure_date"]
}
```

### travel_search_hotels
Search for hotels.
```json
{
  "type": "object",
  "properties": {
    "location": {
      "type": "string"
    },
    "check_in": {
      "type": "string",
      "description": "Check-in date (YYYY-MM-DD)"
    },
    "check_out": {
      "type": "string"
    },
    "guests": {
      "type": "integer",
      "default": 2
    },
    "rooms": {
      "type": "integer",
      "default": 1
    },
    "min_stars": {
      "type": "integer",
      "minimum": 1,
      "maximum": 5
    }
  },
  "required": ["location", "check_in", "check_out"]
}
```

### travel_airport_info
Get airport information.
```json
{
  "type": "object",
  "properties": {
    "code": {
      "type": "string",
      "description": "Airport code"
    }
  },
  "required": ["code"]
}
```

### travel_flight_status
Check flight status.
```json
{
  "type": "object",
  "properties": {
    "flight_number": {
      "type": "string",
      "description": "Flight number (e.g., 'AA100')"
    },
    "date": {
      "type": "string"
    }
  },
  "required": ["flight_number"]
}
```

### travel_create_itinerary
Create a trip itinerary.
```json
{
  "type": "object",
  "properties": {
    "destination": {
      "type": "string"
    },
    "start_date": {
      "type": "string"
    },
    "end_date": {
      "type": "string"
    },
    "interests": {
      "type": "array",
      "items": {"type": "string"},
      "description": "Travel interests (food, history, nature, etc.)"
    }
  },
  "required": ["destination", "start_date", "end_date"]
}
```

### travel_currency_convert
Convert currencies for travel.
```json
{
  "type": "object",
  "properties": {
    "amount": {
      "type": "number"
    },
    "from": {
      "type": "string",
      "description": "Source currency code"
    },
    "to": {
      "type": "string",
      "description": "Target currency code"
    }
  },
  "required": ["amount", "from", "to"]
}
```

## Commands

### search_flights
```bash
curl -s "https://api.skyscanner.net/apiservices/browsequotes/v1.0/US/USD/en-US/{origin}/{destination}/{departure_date}?apiKey=$SKYSCANNER_API_KEY"
```

### currency_convert
```bash
curl -s "https://api.exchangerate-api.com/v4/latest/{from}" | jq ".rates.{to} * {amount}"
```

## Environment
- SKYSCANNER_API_KEY

## Permissions
- network
