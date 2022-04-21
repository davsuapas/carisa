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

package net

import (
	"testing"

	"github.com/carisa/internal/config"
	"github.com/carisa/pkg/log"
	"github.com/hashicorp/consul/api"
	"github.com/hashicorp/consul/sdk/testutil"
	"github.com/hashicorp/serf/testutil/retry"
	"github.com/stretchr/testify/assert"
)

func TestHealthAddress(t *testing.T) {
	type args struct {
		srv    config.Server
		health config.Health
	}
	tests := []struct {
		name  string
		args  args
		panic bool
	}{
		{
			name: "Server address",
			args: args{
				srv: config.Server{
					Address: "srv",
				},
				health: config.Health{
					Port: 8080,
				},
			},
			panic: false,
		},
		{
			name: "Server name is empty",
			args: args{
				srv: config.Server{},
				health: config.Health{
					Port: 8080,
				},
			},
			panic: true,
		},
		{
			name: "Port is equal Zero",
			args: args{
				srv: config.Server{
					Address: "srv",
				},
				health: config.Health{},
			},
			panic: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if tt.panic {
				assert.Panics(t, func() { HealthAddress(tt.args.srv, tt.args.health) }, "Panics")
				return
			}
			address := HealthAddress(tt.args.srv, tt.args.health)
			assert.Equal(t, "srv:8080", address, "Server address")
		})
	}
}

func TestNewConsulDiscovery(t *testing.T) {
	d := NewConsulDiscovery(log.TestLogger(), "")
	assert.NotNil(t, d.log, "Logger")
	assert.NotNil(t, d.client, "Consul client")
}

func TestConsulDiscovery_Register(t *testing.T) {
	t.Parallel()

	ch := newConfigHealth()
	c, s := testConsulServer(t)
	defer closecs(s)
	d := testNewConsulDiscovery(c)

	type args struct {
		srv   config.Server
		healh config.Health
		name  string
	}
	tests := []struct {
		name  string
		args  args
		panic bool
	}{
		{
			name: "Register a service",
			args: args{
				srv: config.Server{
					ID:       "123",
					Address:  "127.0.0.1",
					Port:     8080,
					NodeType: config.Worker,
				},
				healh: ch,
				name:  "ns",
			},
			panic: false,
		},
		{
			name: "GraphID empty",
			args: args{
				srv:   config.Server{},
				healh: newConfigHealth(),
				name:  "",
			},
			panic: true,
		},
		{
			name: "Address equal to empty",
			args: args{
				srv:   config.Server{},
				healh: newConfigHealth(),
				name:  "",
			},
			panic: true,
		},
		{
			name: "Port equal to zero",
			args: args{
				srv: config.Server{
					ID:       "123",
					Address:  "127.0.0.1",
					Port:     0,
					NodeType: config.Worker,
				},
				healh: ch,
				name:  "gi",
			},
			panic: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if tt.panic {
				assert.Panics(t, func() { d.Register(tt.args.srv, tt.args.healh, tt.args.name) }, "Panics")
				return
			}

			d.Register(tt.args.srv, tt.args.healh, tt.args.name)
			rs, _, err := c.Agent().Service(tt.args.srv.ID, &api.QueryOptions{})
			if assert.NoError(t, err, "Error getting service info") {
				assert.Equal(t, tt.args.srv.ID, rs.ID, "ID")
				assert.Equal(t, tt.args.srv.Port, rs.Port, "Port")
				assert.Equal(t, tt.args.srv.NodeType, config.NodeType(rs.Tags[0]), "TypeNode")
				assert.Equal(t, tt.args.name, rs.Service, "GraphID")
			}
		})
	}
}

func TestConsulDiscovery_Deregister(t *testing.T) {
	t.Parallel()

	c, s := testConsulServer(t)
	defer closecs(s)
	d := testNewConsulDiscovery(c)

	srv := config.Server{
		ID:       "123",
		Address:  "192.168.100.1",
		Port:     8080,
		NodeType: config.Worker,
	}

	d.Register(srv, newConfigHealth(), "ns")
	d.Deregister(srv.ID)
	_, _, err := c.Agent().Service(srv.ID, &api.QueryOptions{})
	assert.Contains(t, err.Error(), "404")
}

func testConsulServer(t *testing.T) (*api.Client, *testutil.TestServer) {
	// Skip test when -short flag provided; any tests that create a test
	// server will take at least 100ms which is undesirable for -short
	if testing.Short() {
		t.Skip("too slow for testing.Short")
	}

	// Create server
	var server *testutil.TestServer
	var err error
	retry.RunWith(retry.ThreeTimes(), t, func(r *retry.R) {
		server, err = testutil.NewTestServerConfigT(t, nil)
		if err != nil {
			r.Fatalf("Failed to start server: %v", err.Error())
		}
	})
	if server.Config.Bootstrap {
		server.WaitForLeader(t)
	}

	// Make client config
	conf := api.DefaultConfig()
	conf.Address = server.HTTPAddr

	// Create client
	client, err := api.NewClient(conf)
	if err != nil {
		if err := server.Stop(); err != nil {
			t.Fatalf("err: %v", err)
		}
		t.Fatalf("err: %v", err)
	}

	return client, server
}

func newConfigHealth() config.Health {
	return config.Health{
		Interval:                       60,
		Timeout:                        1,
		FailuresBeforeCritical:         10,
		DeregisterCriticalServiceAfter: 1,
		Port:                           5050,
	}
}

func testNewConsulDiscovery(c *api.Client) Discovery {
	return &ConsulDiscovery{
		log:    log.TestLogger(),
		client: c,
	}
}

func closecs(s *testutil.TestServer) {
	_ = s.Stop()
}
