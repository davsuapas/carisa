/*
 *   Copyright (c) 2022 CARISA
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

package master

import (
	"os"
	"testing"

	"github.com/carisa/internal/config"
	"github.com/stretchr/testify/assert"
)

func TestFactoryBuild(t *testing.T) {
	const ev = "CARISA_MASTER_CONFIG_JSON"

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
					"server": {"id": "id"}
				}`)
			},
			ec: Config{
				Common: config.Default(config.Master, MasterPort),
			},
			panic: false,
		},
		{
			name: "Error unserialize environment variable",
			action: func() {
				os.Unsetenv(ev)
				os.Setenv(ev, `{
					"server": {"id": "id"
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
			tt.ec.Common.ID = f.config.Common.ID
			assert.Equal(t, tt.ec.Common, f.config.Common, "Common")
			assert.Equal(t, "id", f.config.Server.ID, "Server ID")
			assert.NotNil(t, f.discovery, "Discovery")
			assert.NotNil(t, f.health, "Discovery")
			assert.NotNil(t, f.log, "Logger")
		})
	}
}
