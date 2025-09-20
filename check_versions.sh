#!/bin/bash
echo "Checking latest versions from crates.io..."
echo ""

# Function to check version
check_crate() {
    local crate=$1
    local current=$2
    local latest=$(curl -s "https://crates.io/api/v1/crates/$crate" 2>/dev/null | grep -o '"max_version":"[^"]*"' | cut -d'"' -f4)
    
    if [ -z "$latest" ]; then
        echo "$crate: Current: $current, Latest: Unable to fetch"
    else
        echo "$crate: Current: $current, Latest: $latest"
    fi
}

# Check each dependency
check_crate "anyhow" "1.0.99"
check_crate "colored" "2.2.0"
check_crate "indicatif" "0.17.11"
check_crate "walkdir" "2.5.0"
check_crate "tokio" "1.47.1"
check_crate "futures" "0.3.31"
check_crate "clap" "4.5.45"
check_crate "reqwest" "0.11.27"
check_crate "serde" "1.0.225"
check_crate "serde_json" "1.0.145"
check_crate "dirs" "5.0.1"
check_crate "tar" "0.4.44"
check_crate "flate2" "1.1.2"
check_crate "zip" "0.6.6"
