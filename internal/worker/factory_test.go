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

package worker

import (
	"os"
	"testing"

	"github.com/carisa/internal/config"
	"github.com/stretchr/testify/assert"
)

func TestFactoryBuild(t *testing.T) {
	const ev = "CARISA_WORKER_CONFIG_JSON"

	tests := []struct {
		name   string
		action func()
		ec     Config
		panic  bool
	}{
		{
			name: "Environment variable",
			action: func() {
				os.Unsetenv(ev)
				os.Setenv(ev, `{
					"graphID": "gi"
				}`)
			},
			ec: Config{
				GraphID: "gi",
				Common:  config.Default(config.Worker, 0),
			},
			panic: false,
		},
		{
			name: "Error unserialize environment variable",
			action: func() {
				os.Unsetenv(ev)
				os.Setenv(ev, `{
					"graphID": "errror_gi",
				}`)
			},
			panic: true,
		},
	}
	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()
			tt.action()
			if tt.panic {
				assert.Panics(t, func() { FactoryBuild("") })
				return
			}
			f := FactoryBuild("")
			assert.Equal(t, tt.ec.GraphID, f.config.GraphID, "graphID")
			tt.ec.Common.ID = f.config.Common.ID
			assert.Equal(t, tt.ec.Common, f.config.Common, "Common")
			assert.NotNil(t, f.discovery, "Discovery")
			assert.NotNil(t, f.health, "Health")
			assert.NotNil(t, f.log, "Logger")
		})
	}
}
