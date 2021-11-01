/*
 *   Copyright (c) 2021 CARISA
 *   All rights reserved.

 *   Licensed under the Apache License, Version 2.0 (the "License");
 *   you may not use this file except in compliance with the License.
 *   You may obtain a copy of the License at

 *   http://www.apache.org/licenses/LICENSE-2.0

 *   Unless required by applicable law or agreed to in writing, software
 *   distributed under the License is distributed on an "AS IS" BASIS,
 *   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 *   See the License for the specific language governing permissions and
 *   limitations under the License.
 */

package config

import (
	"github.com/rs/xid"
	"go.uber.org/zap/zapcore"
)

// Zap defines the configuration for log framework
type Zap struct {
	// Development mode. Common value: false
	Development bool `json:",omitempty"`
	// Level. See zapcore.Level.
	Level zapcore.Level `json:",omitempty"`
	// Encoding type. Common value: Depending of Development flag
	// The values can be: json -> json format, console -> console format
	Encoding string `json:",omitempty"`
}

// Server defines the server configuration
type Server struct {
	ID       string `json:",omitempty"`
	Address  string `json:",omitempty"`
	Port     int    `json:",omitempty"`
	TypeNode string `json:"-"`
}

// Discovery defines the discovery server
type Discovery struct {
	// Server is the server address
	Server string `json:",omitempty"`
}

// Common defines the common config
type Common struct {
	Zap       `json:"log,omitempty"`
	Discovery `json:"discovery,omitempty"`
	Server    `json:"server,omitempty"`
}

// Default defines the default common config
func Default(typeNode string) Common {
	return Common{
		Zap: Zap{
			Development: true,
			Level:       zapcore.DebugLevel,
			Encoding:    "console",
		},
		Discovery: DefaultDiscovery(),
		Server: Server{
			ID:       xid.New().String(),
			Address:  "",
			Port:     0,
			TypeNode: typeNode,
		},
	}
}

func DefaultDiscovery() Discovery {
	return Discovery{}
}
