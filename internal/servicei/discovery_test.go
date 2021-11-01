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

package servicei

import (
	"testing"

	"github.com/carisa/internal/config"
	"github.com/hashicorp/consul/api"
	"github.com/hashicorp/consul/sdk/testutil"
	"github.com/hashicorp/serf/testutil/retry"
	"github.com/stretchr/testify/assert"
)

func TestNewConsulDiscovery(t *testing.T) {
	d := NewConsulDiscovery(config.TestLogger(), config.DefaultDiscovery())
	assert.NotNil(t, d.log, "Logger")
	assert.NotNil(t, d.client, "Consul client")
}

func TestConsulDiscovery_Register(t *testing.T) {
	t.Parallel()

	c, s := testConsulServer(t)
	defer closecs(s)
	d := testNewConsulDiscovery(c)

	type args struct {
		rs RegisterService
	}
	tests := []struct {
		name  string
		args  args
		panic bool
	}{
		{
			name: "Register a service",
			args: args{
				rs: ConvertToRS(config.Server{
					ID:       "123",
					Port:     8080,
					TypeNode: "worker",
				}, "ns"),
			},
			panic: false,
		},
		{
			name: "Namespace empty",
			args: args{
				rs: ConvertToRS(config.Server{}, ""),
			},
			panic: true,
		},
		{
			name: "Port equal to zero",
			args: args{
				rs: ConvertToRS(config.Server{
					Port: 0,
				}, "ns"),
			},
			panic: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if tt.panic {
				assert.Panics(t, func() { d.Register(tt.args.rs) }, "Panics")
				return
			}

			d.Register(tt.args.rs)
			rs, _, err := c.Agent().Service(tt.args.rs.ID, &api.QueryOptions{})
			if assert.NoError(t, err, "Error getting service info") {
				assert.Equal(t, tt.args.rs.ID, rs.ID, "ID")
				assert.Equal(t, tt.args.rs.Port, rs.Port, "Port")
				assert.Equal(t, tt.args.rs.TypeNode, rs.Tags[0], "TypeNode")
				assert.Equal(t, tt.args.rs.Namespace, rs.Service, "Namespace")
			}
		})
	}
}

func TestConsulDiscovery_Deregister(t *testing.T) {
	t.Parallel()

	c, s := testConsulServer(t)
	defer closecs(s)
	d := testNewConsulDiscovery(c)

	rs := RegisterService{
		ID:        "123",
		Namespace: "ns",
		TypeNode:  "worker",
		Address:   "192.168.100.1",
		Port:      8080,
	}
	d.Register(rs)
	d.Deregister(rs.ID)
	_, _, err := c.Agent().Service(rs.ID, &api.QueryOptions{})
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

func testNewConsulDiscovery(c *api.Client) Discovery {
	return &ConsulDiscovery{
		log:    config.TestLogger(),
		client: c,
	}
}

func closecs(s *testutil.TestServer) {
	_ = s.Stop()
}
